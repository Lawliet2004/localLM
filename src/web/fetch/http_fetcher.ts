/**
 * Native Node transport required: a WebView cannot safely pin DNS or enforce robots.
 * The transport itself owns fetch policy: SSRF-pinned DNS, robots compliance,
 * rate limiting, the GitHub raw-content fast path, and bounded reads.
 */
export interface FetchOptions { timeoutSeconds?: number; maxBytes?: number; maxRedirects?: number; userAgent?: string; maxRetries?: number }
export interface FetchResult { url: string; finalUrl: string; success: boolean; status?: number; body?: string; mimeType?: string; error?: string; durationMs: number; fastPath?: 'github_raw' }
export class HttpFetcher {
  private transport?: Promise<{fetch(url: string, options: FetchOptions): Promise<FetchResult>}>;
  constructor(private globalConcurrency = 8, private perDomainConcurrency = 2) {}
  async fetch(url: string, options: FetchOptions = {}): Promise<FetchResult> {
    try {
      this.transport ??= import(/* @vite-ignore */ '../../../scripts/web-transport.mjs' as string)
        .then(module => new module.PinnedPageFetcher(this.globalConcurrency, this.perDomainConcurrency));
      return await (await this.transport).fetch(url, options);
    } catch (error) {
      return {url, finalUrl:url, success:false, durationMs:0, error:`Native retrieval unavailable: ${String(error)}`};
    }
  }
}
