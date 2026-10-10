import type { ProviderConnection } from './types';

/** Match the native loopback-only inference policy; credentials in URLs are rejected. */
export function localInferenceProvider(provider: Pick<ProviderConnection, 'baseUrl'>): boolean {
  try {
    const url = new URL(provider.baseUrl.trim());
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) return false;
    const host = url.hostname.toLowerCase().replace(/\.$/, '');
    return host === 'localhost' || host === '[::1]' || /^127\.(?:\d{1,3}\.){2}\d{1,3}$/.test(host);
  } catch { return false; }
}
