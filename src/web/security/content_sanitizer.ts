/**
 * Content Sanitizer: Strips potentially dangerous markup (scripts, iframes, objects,
 * event handlers) to ensure citations and page extracts never present an XSS risk.
 */

const DANGEROUS_TAGS_REGEX = /<\/?(script|iframe|object|embed|applet|style|meta|link|base|form|input|button|textarea|select)[^>]*>/gi;
const DANGEROUS_ATTRIBUTES_REGEX = /\s*(on[a-z]+|javascript:|data:text\/html)\s*=[^>\s]*/gi;

export function sanitizeHtml(html: string): string {
  if (!html) return '';
  return html
    .replace(DANGEROUS_TAGS_REGEX, '')
    .replace(DANGEROUS_ATTRIBUTES_REGEX, '')
    .replace(/<svg\b[^>]*>[\s\S]*?<\/svg>/gi, '') // remove SVG which can embed scripts
    .replace(/<!--[\s\S]*?-->/g, ''); // remove comments
}

export function escapeHtml(text: string): string {
  if (!text) return '';
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;');
}
