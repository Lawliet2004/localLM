/**
 * Main Content Extractor: Extracts readable article/page text, strips boilerplate
 * (nav, ads, cookie notices, scripts, footers), preserves heading structure,
 * and generates content fingerprints.
 */

import { extractPageMetadata, type PageMetadata } from './metadata';

export interface ExtractedLink {
  text: string;
  href: string;
}

export interface ExtractionResult {
  title: string;
  author?: string;
  publishedAt?: string;
  canonicalUrl?: string;
  description?: string;
  text: string;
  contentHash: string;
  characters: number;
  confidence: number;
  method: 'article_dom' | 'heuristic_strip' | 'plain_text';
  headings?: string[];
  links?: ExtractedLink[];
  tables?: string[];
}

/**
 * Computes a fast deterministic SHA-256 or djb2 hash string.
 */
export function computeContentHash(text: string): string {
  // Simple, fast 64-bit FNV-1a / djb2 hash representation for cross-environment safety
  let h1 = 0xdeadbeef;
  let h2 = 0x41c6ce57;
  for (let i = 0; i < text.length; i++) {
    const ch = text.charCodeAt(i);
    h1 = Math.imul(h1 ^ ch, 2654435761);
    h2 = Math.imul(h2 ^ ch, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
  return (4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(16);
}

export function extractMainContent(html: string, fallbackTitle: string = ''): ExtractionResult {
  const meta: PageMetadata = extractPageMetadata(html);

  // If already plain text (no HTML tags)
  if (!/<[a-z][\s\S]*>/i.test(html)) {
    const clean = html.trim();
    return {
      title: meta.title || fallbackTitle,
      author: meta.author,
      publishedAt: meta.publishedAt,
      canonicalUrl: meta.canonicalUrl,
      description: meta.description,
      text: clean,
      contentHash: computeContentHash(clean),
      characters: clean.length,
      confidence: 0.9,
      method: 'plain_text',
      headings: [],
      links: [],
      tables: [],
    };
  }

  // 1. Remove non-content tags & boilerplate elements
  let processed = html
    .replace(/<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script>/gi, '')
    .replace(/<style\b[^<]*(?:(?!<\/style>)<[^<]*)*<\/style>/gi, '')
    .replace(/<noscript\b[^<]*(?:(?!<\/noscript>)<[^<]*)*<\/noscript>/gi, '')
    .replace(/<svg\b[^<]*(?:(?!<\/svg>)<[^<]*)*<\/svg>/gi, '')
    .replace(/<!--[\s\S]*?-->/g, '');

  // Strip navigation, header, footer, aside, ads, cookie notices
  const boilerplateTags = ['nav', 'header', 'footer', 'aside', 'form', 'dialog'];
  for (const tag of boilerplateTags) {
    const regex = new RegExp(`<${tag}\\b[^<]*(?:(?!<\\/${tag}>)<[^<]*)*<\\/${tag}>`, 'gi');
    processed = processed.replace(regex, '');
  }

  // Remove common ad/cookie banner classes/ids
  processed = processed.replace(
    /<div[^>]*(id|class)=["'][^"']*(cookie|banner|advertisement|sponsor|popup|modal|consent)[^"']*["'][^<]*(?:(?!<\/div>)<[^<]*)*<\/div>/gi,
    ''
  );

  // 2. Locate <article> or <main> if present
  let articleContent = '';
  const mainOrArticleMatch =
    processed.match(/<article\b[^>]*>([\s\S]*?)<\/article>/i) ||
    processed.match(/<main\b[^>]*>([\s\S]*?)<\/main>/i);

  let method: 'article_dom' | 'heuristic_strip' = 'heuristic_strip';
  if (mainOrArticleMatch && mainOrArticleMatch[1].trim().length > 300) {
    articleContent = mainOrArticleMatch[1];
    method = 'article_dom';
  } else {
    articleContent = processed;
  }

  // 3. Convert HTML structure to formatted Markdown/text.
  // Headings, link targets, and table rows are preserved structurally:
  // headings seed chunk section paths, links enable follow-up reads, and
  // tables keep row/cell alignment instead of collapsing into prose.
  const headings: string[] = [];
  const headingTexts = [...articleContent.matchAll(/<h[1-6]\b[^>]*>([\s\S]*?)<\/h[1-6]>/gi)]
    .map((m) => m[1].replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim())
    .filter(Boolean);
  headings.push(...headingTexts.slice(0, 50));

  const links: ExtractedLink[] = [...articleContent.matchAll(/<a\b[^>]*href=["']([^"']+)["'][^>]*>([\s\S]*?)<\/a>/gi)]
    .map((m) => ({ href: m[1].trim(), text: m[2].replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim().slice(0, 200) }))
    .filter((l) => l.href && /^https?:\/\//i.test(l.href))
    .slice(0, 100);

  const tables: string[] = [...articleContent.matchAll(/<table\b[^>]*>([\s\S]*?)<\/table>/gi)]
    .slice(0, 10)
    .flatMap((m) => [...m[1].matchAll(/<tr\b[^>]*>([\s\S]*?)<\/tr>/gi)].slice(0, 30).map((row) =>
      [...row[1].matchAll(/<t[hd]\b[^>]*>([\s\S]*?)<\/t[hd]>/gi)]
        .map((cell) => cell[1].replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim().slice(0, 300))
        .filter(Boolean).join(' | '),
    ).filter((row) => row.length > 0));

  // Preserve headings
  articleContent = articleContent.replace(/<h1\b[^>]*>([\s\S]*?)<\/h1>/gi, '\n\n# $1\n\n');
  articleContent = articleContent.replace(/<h2\b[^>]*>([\s\S]*?)<\/h2>/gi, '\n\n## $1\n\n');
  articleContent = articleContent.replace(/<h3\b[^>]*>([\s\S]*?)<\/h3>/gi, '\n\n### $1\n\n');
  articleContent = articleContent.replace(/<h[4-6]\b[^>]*>([\s\S]*?)<\/h[4-6]>/gi, '\n\n#### $1\n\n');

  // Convert paragraphs and line breaks
  articleContent = articleContent.replace(/<p\b[^>]*>([\s\S]*?)<\/p>/gi, '\n\n$1\n\n');
  articleContent = articleContent.replace(/<br\s*\/?>/gi, '\n');

  // Convert list items
  articleContent = articleContent.replace(/<li\b[^>]*>([\s\S]*?)<\/li>/gi, '\n- $1');

  // Convert table cells & rows
  articleContent = articleContent.replace(/<th\b[^>]*>([\s\S]*?)<\/th>/gi, ' | $1');
  articleContent = articleContent.replace(/<td\b[^>]*>([\s\S]*?)<\/td>/gi, ' | $1');
  articleContent = articleContent.replace(/<\/tr>/gi, ' |\n');

  // Strip all remaining HTML tags
  let text = articleContent.replace(/<[^>]+>/g, ' ');

  // Decode common HTML entities
  text = text
    .replace(/&nbsp;/gi, ' ')
    .replace(/&amp;/gi, '&')
    .replace(/&lt;/gi, '<')
    .replace(/&gt;/gi, '>')
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/gi, "'")
    .replace(/&#x27;/gi, "'")
    .replace(/&mdash;/gi, '—')
    .replace(/&ndash;/gi, '–');

  // Collapse excess whitespace and empty lines
  const lines = text
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0);

  let cleanText = lines.join('\n\n');
  if (
    meta.articleBody &&
    meta.articleBody.length > 200 &&
    (cleanText.length < 500 || meta.articleBody.length > cleanText.length)
  ) {
    cleanText = meta.articleBody;
    method = 'article_dom';
  }
  const confidence = cleanText.length > 500 ? (method === 'article_dom' ? 0.95 : 0.85) : 0.60;

  return {
    title: meta.title || fallbackTitle,
    author: meta.author,
    publishedAt: meta.publishedAt,
    canonicalUrl: meta.canonicalUrl,
    description: meta.description,
    text: cleanText,
    contentHash: computeContentHash(cleanText),
    characters: cleanText.length,
    confidence,
    method,
    headings,
    links,
    tables,
  };
}
