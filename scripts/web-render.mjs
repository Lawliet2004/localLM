/**
 * Last-resort render for pages a static fetch cannot read.
 *
 * The document and every later hop are retrieved with the pinned HTTP transport.
 * Headless Edge, when it launches, gets an isolated non-persistent context and
 * the bytes we already fetched. It is not navigated at the target URL.
 *
 * Routing is application policy, not an OS sandbox. Anything the route handler
 * does not explicitly fulfill is aborted. If Edge cannot be launched, the
 * result is null — there is no unrestricted browser fallback.
 */
import { lookup as dnsLookup } from 'node:dns/promises';
import { existsSync } from 'node:fs';
import { isIP } from 'node:net';
import { join } from 'node:path';
import { isPublicAddress, pinnedGet } from './web-transport.mjs';

const EDGE_CANDIDATES = [
  process.env.LOCALLM_EDGE_PATH,
  'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
  'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
  process.env.LOCALAPPDATA ? join(process.env.LOCALAPPDATA, 'Microsoft\\Edge\\Application\\msedge.exe') : undefined,
].filter(Boolean);

const REDIRECT_STATUSES = new Set([301, 302, 303, 307, 308]);
const HOP_HEADERS = new Set([
  'connection', 'keep-alive', 'proxy-connection', 'transfer-encoding', 'upgrade',
  'content-encoding', 'content-length', 'content-disposition', 'location',
  'set-cookie', 'set-cookie2', 'proxy-authenticate', 'proxy-authorization',
]);

/**
 * Isolation flags for browser.newContext. Not a persistent profile and not an
 * OS sandbox: a dead proxy makes a request that escapes routing fail closed.
 */
export const ISOLATED_CONTEXT_OPTIONS = Object.freeze({
  serviceWorkers: 'block',
  acceptDownloads: false,
  proxy: { server: 'http://127.0.0.1:1' },
});

let launcherError = null;

export function lastLauncherError() {
  return launcherError;
}

export function findEdgeBinary(candidates = EDGE_CANDIDATES, exists = existsSync) {
  return candidates.find((path) => exists(path)) || null;
}

function stripHost(host) {
  return String(host || '').trim().toLowerCase().replace(/^\[|\]$/g, '').replace(/\.$/, '');
}

function rawHostname(input) {
  const text = String(input || '').trim();
  const scheme = text.match(/^[a-z][a-z0-9+.-]*:(.*)$/i);
  if (!scheme) return '';
  let rest = scheme[1];
  if (!rest.startsWith('//')) return '';
  rest = rest.slice(2);
  const cut = rest.search(/[/?#]/);
  const authority = cut === -1 ? rest : rest.slice(0, cut);
  const at = authority.lastIndexOf('@');
  const hostport = at === -1 ? authority : authority.slice(at + 1);
  if (hostport.startsWith('[')) {
    const end = hostport.indexOf(']');
    return end === -1 ? hostport : hostport.slice(1, end);
  }
  const colon = hostport.lastIndexOf(':');
  if (colon !== -1 && hostport.indexOf(':') === colon) return hostport.slice(0, colon);
  return hostport;
}

function isIpv4Mapped(host) {
  const clean = stripHost(host);
  return clean.startsWith('::ffff:') || /^(?:0+:){5}ffff:/i.test(clean);
}

function isInternalHost(host) {
  return /^(localhost|metadata|instance-data)$|\.(local|internal|localhost|corp|home|lan)$/i.test(host);
}

/** Canonical dotted IPv4 only: four decimal octets, no leading zeros. */
function isCanonicalDottedIpv4(host) {
  return /^(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/.test(host);
}

function isDottedOrNumericIp(host) {
  return /^(?:\d+|0x[0-9a-f]+)(?:\.(?:\d+|0x[0-9a-f]+)){0,3}$/i.test(host);
}

/**
 * Reject decimal, hex, octal, short, and IPv4-mapped spellings before any
 * socket exists. Canonical dotted IPv4 is left for the public-address check.
 */
export function alternateIpReason(rawUrl) {
  let parsed;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return 'invalid url';
  }
  if (parsed.username || parsed.password) return 'credentials blocked';
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return 'protocol blocked';
  const raw = stripHost(rawHostname(rawUrl));
  const canonical = stripHost(parsed.hostname);
  if (!canonical) return 'empty host';
  if (isIpv4Mapped(raw) || isIpv4Mapped(canonical)) return 'IPv4-mapped IPv6 blocked';
  if (raw && isDottedOrNumericIp(raw) && !isCanonicalDottedIpv4(raw)) return 'alternate IP form blocked';
  if (raw && isIP(canonical) && raw !== canonical) return 'alternate IP form blocked';
  return null;
}

/** Static block. Null means the URL may proceed to a fresh lookup. No DNS. */
export function gateUrl(rawUrl) {
  const reason = alternateIpReason(rawUrl);
  if (reason) return reason;
  let parsed;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return 'invalid url';
  }
  const host = stripHost(parsed.hostname);
  if (isInternalHost(host)) return 'internal host blocked';
  if (isIP(host) && !isPublicAddress(host)) return 'Non-public address blocked';
  return null;
}

function headerValue(headers, name) {
  if (!headers) return undefined;
  const wanted = name.toLowerCase();
  for (const [key, value] of Object.entries(headers)) {
    if (key.toLowerCase() === wanted) return Array.isArray(value) ? value[0] : value;
  }
  return undefined;
}

function asBuffer(body) {
  if (body == null) return Buffer.alloc(0);
  if (Buffer.isBuffer(body)) return body;
  if (typeof body === 'string') return Buffer.from(body);
  if (body instanceof Uint8Array) return Buffer.from(body);
  return Buffer.from(String(body));
}

function addressBlockReason(address) {
  const clean = stripHost(address);
  if (isIpv4Mapped(clean)) return 'IPv4-mapped IPv6 blocked';
  if (!isIP(clean) || !isPublicAddress(clean)) return 'Non-public address blocked';
  return null;
}

async function resolvePublic(hostname, lookup, deadlineMs) {
  let timer;
  const pending = Promise.resolve().then(() => lookup(hostname));
  pending.catch(() => {});
  try {
    const addresses = await Promise.race([
      pending,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error('DNS timeout')), Math.min(4000, Math.max(1, deadlineMs)));
      }),
    ]);
    if (!Array.isArray(addresses) || addresses.length === 0) return 'DNS failed';
    for (const entry of addresses) {
      const reason = addressBlockReason(typeof entry === 'string' ? entry : entry?.address);
      if (reason) return reason;
    }
    return null;
  } catch {
    return 'DNS failed';
  } finally {
    clearTimeout(timer);
  }
}

function browserCapabilityReason(request) {
  const type = String(request?.resourceType || '').toLowerCase();
  if (request?.isWebSocket || type === 'websocket') return 'websocket blocked';
  if (request?.isDownload || type === 'download') return 'download blocked';
  if (request?.isPopup) return 'popup blocked';
  if (request?.isServiceWorker || type === 'serviceworker' || type === 'service_worker') return 'service worker blocked';
  return null;
}

function htmlMime(headers) {
  const mime = String(headerValue(headers, 'content-type') || '').split(';')[0].trim().toLowerCase();
  if (!mime) return 'text/html';
  if (mime === 'text/html' || mime === 'application/xhtml+xml') return mime;
  return null;
}

function sanitizeHeaders(headers) {
  const out = {};
  for (const [key, value] of Object.entries(headers || {})) {
    if (HOP_HEADERS.has(key.toLowerCase())) continue;
    if (value == null) continue;
    out[key] = Array.isArray(value) ? value.join(', ') : String(value);
  }
  return out;
}

function escapeAttr(value) {
  return String(value).replace(/[&"'<>]/g, (ch) => `&#${ch.charCodeAt(0)};`);
}

function withBase(html, finalUrl) {
  const tag = `<base href="${escapeAttr(finalUrl)}">`;
  if (/<head[^>]*>/i.test(html)) return html.replace(/<head[^>]*>/i, (match) => `${match}${tag}`);
  if (/<html[^>]*>/i.test(html)) return html.replace(/<html[^>]*>/i, (match) => `${match}<head>${tag}</head>`);
  return `<!doctype html><html><head>${tag}</head><body>${html}</body></html>`;
}

/**
 * One render's HTTP policy. Lookup results are not cached: every hop calls
 * `lookup` again, so a name that flips to a private address is blocked.
 */
export function createMediationSession({
  get = pinnedGet,
  lookup = defaultLookup,
  maxBytes = 5 * 1024 * 1024,
  timeoutMs = 20000,
  maxRedirects = 5,
  maxRequests = 32,
  maxConcurrency = 4,
  now = Date.now,
} = {}) {
  if (!Number.isFinite(maxBytes) || maxBytes < 1) throw new Error('maxBytes must be positive');
  if (!Number.isFinite(timeoutMs) || timeoutMs < 1) throw new Error('timeoutMs must be positive');
  if (!Number.isInteger(maxRedirects) || maxRedirects < 0) throw new Error('maxRedirects must be a non-negative integer');
  if (!Number.isInteger(maxRequests) || maxRequests < 1) throw new Error('maxRequests must be a positive integer');
  if (!Number.isInteger(maxConcurrency) || maxConcurrency < 1) throw new Error('maxConcurrency must be a positive integer');

  const started = now();
  let bytes = 0;
  let requests = 0;
  let inFlight = 0;

  function remainingMs() {
    return timeoutMs - (now() - started);
  }

  function reserve() {
    if (remainingMs() <= 0) return 'time limit';
    if (requests >= maxRequests) return 'request limit';
    if (inFlight >= maxConcurrency) return 'concurrency limit';
    requests += 1;
    inFlight += 1;
    return null;
  }

  async function classify(target) {
    const textual = typeof target === 'string' ? target : target?.href;
    if (!textual) return { allow: false, reason: 'invalid url' };
    const literal = gateUrl(textual);
    if (literal) return { allow: false, reason: literal };
    let parsed;
    try {
      parsed = new URL(textual);
    } catch {
      return { allow: false, reason: 'invalid url' };
    }
    const host = stripHost(parsed.hostname);
    if (!isIP(host)) {
      const reason = await resolvePublic(host, lookup, remainingMs());
      if (reason) return { allow: false, reason, url: parsed };
    }
    return { allow: true, url: parsed };
  }

  async function exchange(rawUrl) {
    let current = String(rawUrl);
    for (let hop = 0; hop <= maxRedirects; hop++) {
      const limited = reserve();
      if (limited) return { ok: false, error: limited, finalUrl: safeHref(current) };
      let redirectTo = null;
      try {
        const decision = await classify(current);
        if (!decision.allow) return { ok: false, error: decision.reason, finalUrl: decision.url?.href || safeHref(current) };
        const parsed = decision.url;
        let response;
        try {
          response = await get(parsed, {
            maxBytes: Math.max(1, maxBytes - bytes),
            timeoutSeconds: Math.max(1, Math.ceil(remainingMs() / 1000)),
          });
        } catch (error) {
          return { ok: false, error: error?.message || 'fetch failed', finalUrl: parsed.href };
        }
        if (remainingMs() <= 0) return { ok: false, error: 'time limit', finalUrl: parsed.href };
        const body = asBuffer(response?.body);
        bytes += body.length;
        if (bytes > maxBytes) return { ok: false, error: 'byte limit', finalUrl: parsed.href };
        if (REDIRECT_STATUSES.has(Number(response?.status))) {
          const location = headerValue(response.headers, 'location');
          if (!location || !String(location).trim()) return { ok: false, error: 'Redirect missing Location', finalUrl: parsed.href };
          const next = nextHop(String(location).trim(), parsed);
          if (next.error) return { ok: false, error: next.error, finalUrl: parsed.href };
          redirectTo = next.href;
        } else {
          return {
            ok: true,
            finalUrl: parsed.href,
            origin: parsed.origin,
            status: Number(response?.status) || 0,
            headers: response?.headers || {},
            body,
          };
        }
      } finally {
        inFlight -= 1;
      }
      current = redirectTo;
    }
    return { ok: false, error: 'Redirect limit exceeded', finalUrl: safeHref(current) };
  }

  async function mediate(request) {
    const capability = browserCapabilityReason(request);
    if (capability) return { action: 'abort', reason: capability };
    const outcome = await exchange(request.url);
    if (!outcome.ok) return { action: 'abort', reason: outcome.error, finalUrl: outcome.finalUrl };
    if (/attachment/i.test(String(headerValue(outcome.headers, 'content-disposition') || ''))) {
      return { action: 'abort', reason: 'download blocked', finalUrl: outcome.finalUrl };
    }
    const mime = htmlMime(outcome.headers) || String(headerValue(outcome.headers, 'content-type') || 'application/octet-stream').split(';')[0].trim();
    return {
      action: 'fulfill',
      reason: 'mediated',
      finalUrl: outcome.finalUrl,
      origin: outcome.origin,
      fulfillment: {
        status: outcome.status || 200,
        headers: sanitizeHeaders(outcome.headers),
        body: outcome.body,
        contentType: mime || 'application/octet-stream',
      },
    };
  }

  async function handleRoute(route, mainPage) {
    const request = route.request();
    const described = describeRequest(request, mainPage);
    let decision;
    try {
      decision = await mediate(described);
    } catch (error) {
      decision = { action: 'abort', reason: error?.message || 'mediation failed' };
    }
    try {
      if (decision.action === 'fulfill') await route.fulfill(decision.fulfillment);
      else await route.abort('blockedbyclient');
    } catch {
      try { await route.abort('blockedbyclient'); } catch { /* already settled */ }
    }
    return decision;
  }

  return { exchange, mediate, handleRoute, remainingMs };
}

function safeHref(value) {
  try {
    return new URL(value).href;
  } catch {
    return String(value);
  }
}

function nextHop(location, base) {
  const absolute = /^[a-z][a-z0-9+.-]*:/i.test(location)
    ? location
    : location.startsWith('//')
      ? `${base.protocol}${location}`
      : null;
  if (absolute) {
    const reason = alternateIpReason(absolute);
    if (reason) return { error: reason };
  }
  try {
    return { href: new URL(location, base).href };
  } catch {
    return { error: 'invalid url' };
  }
}

function describeRequest(request, mainPage) {
  const resourceType = typeof request.resourceType === 'function' ? request.resourceType() : request.resourceType;
  const url = typeof request.url === 'function' ? request.url() : request.url;
  let isPopup = Boolean(request.isPopup);
  try {
    const page = request.frame?.()?.page?.();
    if (page && mainPage && page !== mainPage) isPopup = true;
  } catch { /* plain test double */ }
  const disposition = typeof request.headers === 'function' ? request.headers()['content-disposition'] : request.headers?.['content-disposition'];
  return {
    url,
    resourceType,
    isPopup,
    isWebSocket: request.isWebSocket || String(resourceType).toLowerCase() === 'websocket',
    isDownload: Boolean(request.isDownload) || /attachment/i.test(String(disposition || '')),
    isServiceWorker: Boolean(request.isServiceWorker),
  };
}

function defaultLookup(hostname) {
  return dnsLookup(hostname, { all: true });
}

async function defaultLaunchBrowser({ executablePath, timeoutMs }) {
  const { chromium } = await import('playwright');
  return chromium.launch({
    executablePath,
    headless: true,
    timeout: Math.min(timeoutMs || 20000, 20000),
    args: [
      '--disable-gpu',
      '--disable-extensions',
      '--no-first-run',
      '--no-default-browser-check',
      '--disable-background-networking',
      '--disable-sync',
      '--disable-component-update',
      '--mute-audio',
      '--disable-features=Translate,BackForwardCache,MediaRouter,DialMediaRouteProvider',
    ],
  });
}

async function closeQuiet(resource) {
  try {
    await resource?.close?.();
  } catch { /* already closed */ }
}

async function renderIsolated(browser, session, mediated, virtualTimeBudgetMs) {
  // newContext is in-memory. A persistent profile is never created.
  const context = await browser.newContext({
    ...ISOLATED_CONTEXT_OPTIONS,
    proxy: { ...ISOLATED_CONTEXT_OPTIONS.proxy },
  });
  try {
    const page = await context.newPage();
    page.on?.('popup', (popup) => { void popup?.close?.()?.catch?.(() => {}); });
    page.on?.('download', (download) => { void download?.cancel?.()?.catch?.(() => {}); });
    context.on?.('page', (extra) => { if (extra !== page) void extra?.close?.()?.catch?.(() => {}); });
    await context.route?.('**/*', (route) => session.handleRoute(route, page));
    if (context.routeWebSocket) {
      await context.routeWebSocket('**/*', (socket) => { socket.close(); });
    }
    const timeout = Math.max(1, session.remainingMs());
    await page.setContent(withBase(mediated.body.toString('utf8'), mediated.finalUrl), {
      waitUntil: 'domcontentloaded',
      timeout,
    });
    if (page.waitForLoadState) {
      await page.waitForLoadState('networkidle', { timeout: virtualTimeBudgetMs }).catch(() => {});
    }
    return page.content();
  } finally {
    await closeQuiet(context);
  }
}

function isRenderableDocument(mediated) {
  return mediated.ok
    && mediated.status === 200
    && mediated.body?.length > 0
    && htmlMime(mediated.headers)
    && !/attachment/i.test(String(headerValue(mediated.headers, 'content-disposition') || ''));
}

/**
 * Render `url` and return the serialized DOM plus the post-redirect URL.
 * @returns {Promise<{html: string, finalUrl: string, origin: string} | null>}
 */
export async function renderPage(url, {
  timeoutMs = 20000,
  maxBytes = 5 * 1024 * 1024,
  virtualTimeBudgetMs = 8000,
  maxRedirects = 5,
  maxRequests = 32,
  maxConcurrency = 4,
  edgePath,
  exists = existsSync,
  lookup = defaultLookup,
  get = pinnedGet,
  launchBrowser = defaultLaunchBrowser,
  now = Date.now,
} = {}) {
  launcherError = null;
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    return null;
  }
  if (gateUrl(String(url)) || gateUrl(parsed.href)) return null;

  const binary = edgePath || findEdgeBinary(undefined, exists);
  if (!binary || !exists(binary)) return null;

  const session = createMediationSession({
    get, lookup, maxBytes, timeoutMs, maxRedirects, maxRequests, maxConcurrency, now,
  });
  let mediated;
  try {
    mediated = await session.exchange(String(url));
  } catch {
    return null;
  }
  if (!isRenderableDocument(mediated)) return null;

  let browser;
  try {
    browser = await launchBrowser({ executablePath: binary, timeoutMs });
  } catch (error) {
    launcherError = error instanceof Error ? error.message : String(error);
    return null;
  }
  try {
    const html = await renderIsolated(browser, session, mediated, virtualTimeBudgetMs);
    if (!html || html.length < 150) return null;
    return { html, finalUrl: mediated.finalUrl, origin: mediated.origin };
  } catch {
    return null;
  } finally {
    await closeQuiet(browser);
  }
}
