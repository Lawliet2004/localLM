/**
 * SSRF Guard: Protects against Server-Side Request Forgery by strictly blocking
 * internal, private, loopback, link-local, metadata addresses, protocols, and
 * hostnames that resolve via DNS to private/loopback addresses.
 */

type DnsLookupAddress = { address: string; family: number };
type DnsPromisesModule = {
  lookup: (hostname: string, options: { all: boolean }) => Promise<DnsLookupAddress[]>;
};

export class SSRFError extends Error {
  constructor(message: string) {
    super(`SSRF Protection Blocked: ${message}`);
    this.name = 'SSRFError';
  }
}

// Private IPv4 ranges:
// 10.0.0.0 - 10.255.255.255 (10.0.0.0/8)
// 172.16.0.0 - 172.31.255.255 (172.16.0.0/12)
// 192.168.0.0 - 192.168.255.255 (192.168.0.0/16)
// 127.0.0.0 - 127.255.255.255 (127.0.0.0/8) Loopback
// 169.254.0.0 - 169.254.255.255 (169.254.0.0/16) Link-local & cloud metadata
// 0.0.0.0 - 0.255.255.255 (0.0.0.0/8) Current network
// 100.64.0.0 - 100.127.255.255 (Shared transition / Carrier-grade NAT)
// 192.0.0.0 - 192.0.0.255 (IETF Protocol Assignments)
// 198.18.0.0 - 198.19.255.255 (Benchmark testing)
// 224.0.0.0+ (Multicast & reserved)

export function isPrivateIpv4(ip: string): boolean {
  const parts = ip.split('.').map(Number);
  if (parts.length !== 4 || parts.some((p) => isNaN(p) || p < 0 || p > 255)) {
    return false;
  }

  const [a, b] = parts;

  // 0.0.0.0/8
  if (a === 0) return true;
  // 127.0.0.0/8
  if (a === 127) return true;
  // 10.0.0.0/8
  if (a === 10) return true;
  // 172.16.0.0/12 (172.16.0.0 - 172.31.255.255)
  if (a === 172 && b >= 16 && b <= 31) return true;
  // 192.168.0.0/16
  if (a === 192 && b === 168) return true;
  // 169.254.0.0/16 (Link local / AWS/GCP/Azure metadata)
  if (a === 169 && b === 254) return true;
  // 100.64.0.0/10
  if (a === 100 && b >= 64 && b <= 127) return true;
  // 192.0.0.0/24
  if (a === 192 && b === 0 && parts[2] === 0) return true;
  // 198.18.0.0/15
  if (a === 198 && (b === 18 || b === 19)) return true;
  // 224.0.0.0/4 Multicast or reserved
  if (a >= 224) return true;

  return false;
}

export function isPrivateIpv6(hostname: string): boolean {
  let clean = hostname.toLowerCase();
  if (clean.startsWith('[') && clean.endsWith(']')) {
    clean = clean.slice(1, -1);
  }

  // Loopback ::1
  if (clean === '::1' || clean === '0000:0000:0000:0000:0000:0000:0000:0001') {
    return true;
  }
  // Unspecified ::
  if (clean === '::' || clean === '0000:0000:0000:0000:0000:0000:0000:0000') {
    return true;
  }
  // Unique local fc00::/7 (fc00:: - fdff::)
  if (clean.startsWith('fc') || clean.startsWith('fd')) {
    return true;
  }
  // Link-local fe80::/10 (fe80:: - febf::)
  if (clean.startsWith('fe8') || clean.startsWith('fe9') || clean.startsWith('fea') || clean.startsWith('feb')) {
    return true;
  }
  // IPv4-mapped IPv6 (::ffff:127.0.0.1, etc.)
  if (clean.startsWith('::ffff:')) {
    return true;
  }

  return false;
}

const BLOCKED_HOSTNAMES = new Set([
  'localhost',
  'localhost.localdomain',
  'metadata.google.internal',
  'instance-data',
  'metadata',
]);

const BLOCKED_EXTENSIONS = ['.local', '.internal', '.localhost', '.corp', '.home', '.lan'];

export function validateSafeUrl(rawUrl: string): URL {
  let parsed: URL;
  try {
    parsed = new URL(rawUrl);
  } catch {
    throw new SSRFError(`Malformed URL: "${rawUrl}"`);
  }

  // Scheme must be strictly http or https
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new SSRFError(`Disallowed protocol "${parsed.protocol}". Only HTTP and HTTPS are permitted.`);
  }

  const hostname = parsed.hostname.toLowerCase().trim().replace(/\.$/, '');
  if (parsed.username || parsed.password) throw new SSRFError('URL credentials are prohibited');

  if (!hostname) {
    throw new SSRFError('Empty hostname');
  }

  // Check blocked hostnames
  if (BLOCKED_HOSTNAMES.has(hostname)) {
    throw new SSRFError(`Host "${hostname}" is prohibited.`);
  }

  // Check internal TLDs
  for (const ext of BLOCKED_EXTENSIONS) {
    if (hostname.endsWith(ext)) {
      throw new SSRFError(`Internal network domain "${hostname}" is prohibited.`);
    }
  }

  // Check IPv4 addresses
  if (isPrivateIpv4(hostname)) {
    throw new SSRFError(`Prohibited private or loopback IP address: "${hostname}"`);
  }

  // Check numeric IP representation evasion (e.g. 2130706433 = 127.0.0.1, 0177.0.0.1 octal, hex 0x7f000001)
  if (/^\d+$/.test(hostname) || /^0x[0-9a-f]+$/i.test(hostname)) {
    throw new SSRFError(`Integer/hex IP formats are prohibited: "${hostname}"`);
  }

  // Check IPv6 addresses
  if (hostname.includes(':') || hostname.startsWith('[')) {
    if (isPrivateIpv6(hostname)) {
      throw new SSRFError(`Prohibited IPv6 address: "${hostname}"`);
    }
  }

  return parsed;
}

/**
 * Synchronously checks whether a URL is safe to fetch. Returns true or throws SSRFError.
 */
export function assertSafeUrl(rawUrl: string): boolean {
  validateSafeUrl(rawUrl);
  return true;
}

/**
 * Non-throwing synchronous safe URL check.
 */
export function isSafeUrl(rawUrl: string): boolean {
  try {
    validateSafeUrl(rawUrl);
    return true;
  } catch {
    return false;
  }
}

export type DnsLookupFn = (hostname: string) => Promise<Array<{ address: string; family: number }>>;

let customDnsLookup: DnsLookupFn | null = null;

export function setCustomDnsLookup(lookup: DnsLookupFn | null): void {
  customDnsLookup = lookup;
}

/**
 * Asynchronously checks URL and resolves DNS to verify that destination IPs
 * are not loopback, link-local, cloud metadata, or private network addresses.
 */
export async function assertSafeUrlAsync(rawUrl: string): Promise<boolean> {
  const parsed = validateSafeUrl(rawUrl);
  const hostname = parsed.hostname.toLowerCase().trim();

  // If already recognized as an IP, validateSafeUrl checked it
  if (isPrivateIpv4(hostname)) {
    throw new SSRFError(`Prohibited private or loopback IP address: "${hostname}"`);
  }
  if (hostname.includes(':') && isPrivateIpv6(hostname)) {
    throw new SSRFError(`Prohibited IPv6 address: "${hostname}"`);
  }

  try {
    let addresses: Array<{ address: string; family: number }>;
    if (customDnsLookup) {
      addresses = await customDnsLookup(hostname);
    } else {
      try {
        const dnsMod = (await import(/* @vite-ignore */ 'node:dns/promises' as string)) as DnsPromisesModule;
        addresses = await dnsMod.lookup(hostname, { all: true });
      } catch {
        addresses = [];
      }
    }

    for (const record of addresses) {
      if (record.family === 4 && isPrivateIpv4(record.address)) {
        throw new SSRFError(`Domain "${hostname}" resolved to prohibited private IP "${record.address}"`);
      }
      if (record.family === 6 && isPrivateIpv6(record.address)) {
        throw new SSRFError(`Domain "${hostname}" resolved to prohibited private IPv6 "${record.address}"`);
      }
    }
  } catch (err: any) {
    if (err instanceof SSRFError) {
      throw err;
    }
    // DNS resolution failure (domain doesn't exist or offline) is not an SSRF block
  }

  return true;
}

/**
 * Non-throwing asynchronous safe URL check with DNS resolution.
 */
export async function isSafeUrlAsync(rawUrl: string): Promise<boolean> {
  try {
    await assertSafeUrlAsync(rawUrl);
    return true;
  } catch {
    return false;
  }
}
