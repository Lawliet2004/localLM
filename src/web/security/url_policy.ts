/**
 * URL Policy: Enforces protocol restrictions and validates target URLs.
 */

import { isSafeUrl } from './ssrf_guard';

export function isAllowedProtocol(urlStr: string): boolean {
  try {
    const url = new URL(urlStr);
    return url.protocol === 'http:' || url.protocol === 'https:';
  } catch {
    return false;
  }
}

export function isFetchableUrl(urlStr: string): boolean {
  if (!isAllowedProtocol(urlStr)) return false;
  return isSafeUrl(urlStr);
}
