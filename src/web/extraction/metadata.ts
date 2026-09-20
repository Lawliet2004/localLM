/**
 * Metadata Extractor: Parses OpenGraph, JSON-LD, Twitter Card, and standard HTML meta
 * tags to retrieve publication date, author, description, and canonical URL.
 */

export interface PageMetadata {
  title?: string;
  author?: string;
  publishedAt?: string;
  description?: string;
  canonicalUrl?: string;
  /** Schema.org Article body when publishers ship it in JSON-LD. */
  articleBody?: string;
}

export function extractPageMetadata(html: string): PageMetadata {
  const meta: PageMetadata = {};

  // 1. Canonical URL
  const canonicalMatch = html.match(/<link\s+[^>]*rel=["']canonical["'][^>]*href=["']([^"']+)["']/i) ||
    html.match(/<link\s+[^>]*href=["']([^"']+)["'][^>]*rel=["']canonical["']/i);
  if (canonicalMatch) {
    meta.canonicalUrl = canonicalMatch[1].trim();
  }

  // 2. OpenGraph / Meta Title
  const ogTitleMatch = html.match(/<meta\s+[^>]*property=["']og:title["'][^>]*content=["']([^"']+)["']/i) ||
    html.match(/<meta\s+[^>]*content=["']([^"']+)["'][^>]*property=["']og:title["']/i);
  if (ogTitleMatch) {
    meta.title = ogTitleMatch[1].trim();
  } else {
    const titleTagMatch = html.match(/<title[^>]*>([\s\S]*?)<\/title>/i);
    if (titleTagMatch) {
      meta.title = titleTagMatch[1].replace(/\s+/g, ' ').trim();
    } else {
      const h1Match = html.match(/<h1[^>]*>([\s\S]*?)<\/h1>/i);
      if (h1Match) {
        meta.title = h1Match[1].replace(/<[^>]+>/g, '').replace(/\s+/g, ' ').trim();
      }
    }
  }

  // 3. Description
  const descMatch = html.match(/<meta\s+[^>]*name=["']description["'][^>]*content=["']([^"']+)["']/i) ||
    html.match(/<meta\s+[^>]*property=["']og:description["'][^>]*content=["']([^"']+)["']/i);
  if (descMatch) {
    meta.description = descMatch[1].trim();
  }

  // 4. Author
  const authorMatch = html.match(/<meta\s+[^>]*name=["']author["'][^>]*content=["']([^"']+)["']/i) ||
    html.match(/<meta\s+[^>]*property=["']article:author["'][^>]*content=["']([^"']+)["']/i);
  if (authorMatch) {
    meta.author = authorMatch[1].trim();
  }

  // 5. Published Date from OpenGraph / Article meta
  const dateMatch =
    html.match(/<meta\s+[^>]*property=["']article:published_time["'][^>]*content=["']([^"']+)["']/i) ||
    html.match(/<meta\s+[^>]*name=["']publish-date["'][^>]*content=["']([^"']+)["']/i) ||
    html.match(/<meta\s+[^>]*name=["']date["'][^>]*content=["']([^"']+)["']/i) ||
    html.match(/<meta\s+[^>]*itemprop=["']datePublished["'][^>]*content=["']([^"']+)["']/i);

  if (dateMatch) {
    meta.publishedAt = dateMatch[1].trim();
  }

  // 6. JSON-LD parsing fallback for author and datePublished
  const jsonLdMatches = html.matchAll(/<script\s+[^>]*type=["']application\/ld\+json["'][^>]*>([\s\S]*?)<\/script>/gi);
  for (const match of jsonLdMatches) {
    try {
      const parsed = JSON.parse(match[1]);
      const items = Array.isArray(parsed) ? parsed : [parsed];
      for (const item of items) {
        if (!meta.publishedAt && item.datePublished) {
          meta.publishedAt = String(item.datePublished);
        }
        if (!meta.author && item.author) {
          if (typeof item.author === 'string') meta.author = item.author;
          else if (item.author.name) meta.author = item.author.name;
        }
        if (!meta.title && item.headline) {
          meta.title = String(item.headline);
        }
        if (!meta.articleBody && typeof item.articleBody === 'string' && item.articleBody.trim().length > 200) {
          meta.articleBody = item.articleBody.trim();
        }
      }
    } catch {
      // Ignore JSON parse errors in script tags
    }
  }

  return meta;
}
