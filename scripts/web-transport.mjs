import { lookup } from 'node:dns/promises';
import { BlockList, isIP } from 'node:net';
import http from 'node:http';
import https from 'node:https';
import { createGunzip, createBrotliDecompress, createInflate } from 'node:zlib';
import { RobotsChecker } from '../src/web/fetch/robots_policy.ts';
import { BoundedConcurrencyLimiter } from '../src/web/fetch/rate_limiter.ts';

const denied = new BlockList();
for (const [address, prefix] of [['0.0.0.0',8],['10.0.0.0',8],['100.64.0.0',10],['127.0.0.0',8],['169.254.0.0',16],['172.16.0.0',12],['192.0.0.0',24],['192.0.2.0',24],['192.168.0.0',16],['198.18.0.0',15],['198.51.100.0',24],['203.0.113.0',24],['224.0.0.0',3]]) denied.addSubnet(address, prefix, 'ipv4');
const globalV6 = new BlockList();
globalV6.addSubnet('2000::', 3, 'ipv6');
denied.addSubnet('2001::', 23, 'ipv6');
denied.addSubnet('2001:db8::', 32, 'ipv6');
denied.addSubnet('2002::', 16, 'ipv6');
export function isPublicAddress(ip) {
  const family = isIP(ip);
  return family === 4 ? !denied.check(ip, 'ipv4')
    : family === 6 && globalV6.check(ip, 'ipv6') && !denied.check(ip, 'ipv6');
}

const USER_AGENTS = [
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36',
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36',
  'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:126.0) Gecko/20100101 Firefox/126.0',
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15',
  'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36',
];

export async function addressesFor(url) {
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) throw new Error('Only credential-free HTTP(S) URLs are allowed');
  const host = url.hostname.replace(/^\[|\]$/g, '').replace(/\.$/, '');
  if (/^(localhost|metadata|instance-data)$|\.(local|internal|localhost|corp|home|lan)$/i.test(host)) throw new Error('Internal host blocked');
  let timer;
  try {
    const addresses = isIP(host) ? [{address: host, family: isIP(host)}] : await Promise.race([
      lookup(host, {all: true}),
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error('DNS timeout')), 4000); }),
    ]);
    if (!addresses.length || addresses.some(a => !isPublicAddress(a.address))) throw new Error('Non-public address blocked');
    return addresses;
  } finally { clearTimeout(timer); }
}

/** DNS is resolved once, checked, then pinned into the actual socket lookup. */
export async function pinnedGet(url, options) {
  const addresses = await addressesFor(url);
  return new Promise((resolve, reject) => {
    const request = (url.protocol === 'https:' ? https : http).get(url, {
      agent: false,
      lookup: (_host, opts, callback) => opts.all ? callback(null, addresses) : callback(null, addresses[0].address, addresses[0].family),
      headers: {
        'User-Agent': options.userAgent || USER_AGENTS[Math.floor(Math.random() * USER_AGENTS.length)],
        'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8',
        'Accept-Language': 'en-US,en;q=0.9',
        'Accept-Encoding': 'identity',
        'Sec-Fetch-Dest': 'document',
        'Sec-Fetch-Mode': 'navigate',
        'Sec-Fetch-Site': 'none',
        'Sec-Fetch-User': '?1',
        'Upgrade-Insecure-Requests': '1',
      },
    }, response => {
      if (Number(response.headers['content-length']) > options.maxBytes) { request.destroy(new Error('Response exceeds byte limit')); return; }
      let size = 0;
      const parts = [];
      const encoding = response.headers['content-encoding'];
      const decoder = encoding === 'gzip' ? createGunzip() : encoding === 'br' ? createBrotliDecompress() : encoding === 'deflate' ? createInflate() : null;
      const stream = decoder ? response.pipe(decoder) : response;
      stream.on('data', chunk => {
        size += chunk.length;
        if (size > options.maxBytes) request.destroy(new Error('Response exceeds byte limit'));
        else parts.push(chunk);
      });
      response.on('error', reject);
      stream.on('error', reject);
      stream.on('end', () => resolve({ status: response.statusCode, headers: response.headers, body: Buffer.concat(parts) }));
    });
    const timer = setTimeout(() => request.destroy(new Error('Fetch deadline exceeded')), options.timeoutSeconds * 1000);
    request.on('error', reject);
    request.on('close', () => clearTimeout(timer));
  });
}

export class PinnedPageFetcher {
  constructor(globalConcurrency = 8, perDomainConcurrency = 2, get = pinnedGet) {
    this.get = get; this.limiter = new BoundedConcurrencyLimiter(globalConcurrency, perDomainConcurrency);
    this.robots = new Map(); this.queues = new Map(); this.cooldowns = new Map();
  }
  async limited(url, options) {
    const domain = url.hostname;
    const previous = this.queues.get(domain) || Promise.resolve();
    const task = previous.catch(() => {}).then(async () => {
      if ((this.cooldowns.get(domain) || 0) > Date.now()) throw new Error('Host Retry-After cooldown active');
      let lastError;
      for (let attempt = 0; attempt < 3; attempt++) {
        try {
          // One request per host per second, including robots requests.
          await new Promise(resolve => setTimeout(resolve, 1000));
          const release = await this.limiter.acquire(domain);
          let response;
          try { response = await this.get(url, options); } finally { release(); }
          if ([429, 503].includes(response.status)) {
            const value = response.headers['retry-after'];
            const deadline = /^\d+$/.test(value || '') ? Date.now() + Number(value) * 1000 : Date.parse(value);
            this.cooldowns.set(domain, Math.max(Date.now() + 30000, Number.isFinite(deadline) ? deadline : 0));
            lastError = new Error(`HTTP ${response.status}`);
            if (attempt < 2) { await new Promise(r => setTimeout(r, 3000)); continue; }
            throw lastError;
          }
          return response;
        } catch (err) {
          lastError = err;
          if (err.message === 'Host Retry-After cooldown active') throw err;
          if (attempt < 2) { await new Promise(r => setTimeout(r, 3000)); continue; }
          throw err;
        }
      }
      throw lastError;
    });
    this.queues.set(domain, task);
    return task;
  }
  async allowed(url, options) {
    if (!this.robots.has(url.origin)) {
      const task = (async () => {
        const res = await this.limited(new URL('/robots.txt', url), {...options, maxBytes: 512000});
        if (res.status === 404 || res.status === 410) return [];
        if (res.status !== 200) return []; // Softened: allow if robots.txt unavailable
        return new RobotsChecker().parseRobotsTxt(res.body.toString('utf8'));
      })();
      this.robots.set(url.origin, task);
    }
    return new RobotsChecker().isPathAllowed(url.pathname + url.search, await this.robots.get(url.origin), 'LocalLM-Research');
  }
  async fetch(raw, overrides = {}) {
    const options = {timeoutSeconds: 10, maxBytes: 5 * 1024 * 1024, maxRedirects: 5, ...overrides};
    const started = Date.now();
    let url;
    try {
      url = new URL(raw);
      for (let hop = 0; hop <= options.maxRedirects; hop++) {
        await addressesFor(url);
        if (!await this.allowed(url, options)) throw new Error('robots.txt disallows this page');
        const res = await this.limited(url, options);
        if ([301,302,303,307,308].includes(res.status)) {
          if (!res.headers.location) throw new Error('Redirect missing Location');
          url = new URL(res.headers.location, url);
          continue;
        }
        if (res.status !== 200) throw new Error(`Page unavailable: HTTP ${res.status}`);
        const mime = String(res.headers['content-type'] || '').split(';')[0].trim().toLowerCase();
        if (!['text/html','text/plain','text/markdown','application/xhtml+xml','application/json','application/pdf'].includes(mime)) throw new Error(`Unsupported MIME ${mime}`);
        // PDFs travel as latin1 text so the worker's extractor sees raw bytes.
        const body = mime === 'application/pdf' ? res.body.toString('latin1') : res.body.toString('utf8');
        return {url: raw, finalUrl: url.href, success: true, status: res.status, body, mimeType: mime, durationMs: Date.now() - started};
      }
      throw new Error('Redirect limit exceeded');
    } catch (error) {
      return {url: raw, finalUrl: url?.href || raw, success: false, error: String(error.message), durationMs: Date.now() - started};
    }
  }
}