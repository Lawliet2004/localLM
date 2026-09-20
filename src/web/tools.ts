/**
 * Web Agent Tools: Exposes progressive disclosure tools (web_search, web_open, web_find, weather)
 * to local models while keeping context bounded and secure.
 *
 * Section 91 & Section 92:
 * SEARCH -> see result summaries
 * OPEN -> see extracted page
 * FIND -> see specific relevant passage
 */

import { WebSearchEngine } from './index';
import { chunkDocument } from './chunking/semantic_chunker';
import { InMemoryBM25 } from './retrieval/bm25';
import type { RetrievedDocument } from './types';

export interface ToolDefinition {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
}

export class WebAgentTools {
  private openedDocuments = new Map<string, RetrievedDocument>();

  constructor(private engine: WebSearchEngine = new WebSearchEngine()) {}

  getToolDefinitions(): ToolDefinition[] {
    return [
      {
        name: 'web_search',
        description: 'Search the web for current information, official documentation, or facts outside your training data.',
        parameters: {
          type: 'object',
          properties: {
            query: { type: 'string', description: 'Search terms to query' },
            freshness: {
              type: 'string',
              enum: ['day', 'week', 'month', 'year', 'any'],
              description: 'Freshness requirement',
            },
          },
          required: ['query'],
        },
      },
      {
        name: 'weather',
        description: 'Get current weather report and forecast for any city or location.',
        parameters: {
          type: 'object',
          properties: {
            location: { type: 'string', description: 'City or location name, e.g. "Ranaghat", "Tokyo"' },
          },
          required: ['location'],
        },
      },
      {
        name: 'web_open',
        description: 'Open a specific URL from search results to read its extracted main text.',
        parameters: {
          type: 'object',
          properties: {
            url: { type: 'string', description: 'URL of the web page to open' },
          },
          required: ['url'],
        },
      },
      {
        name: 'web_find',
        description: 'Search for a specific keyword or phrase within a previously opened webpage.',
        parameters: {
          type: 'object',
          properties: {
            url: { type: 'string', description: 'URL of the page' },
            query: { type: 'string', description: 'Specific phrase or question to find in the page' },
          },
          required: ['url', 'query'],
        },
      },
    ];
  }

  async executeTool(name: string, args: Record<string, any>): Promise<string> {
    if (name === 'weather') {
      const loc = args.location || 'Ranaghat';
      const session = await this.engine.research(`weather in ${loc}`);
      return session.answer || 'No weather data found.';
    }

    if (name === 'web_search') {
      const q = args.query;
      const session = await this.engine.research(q);
      // Return concise SERP summaries (Progressive Disclosure Step 1: SEARCH)
      if (session.results.length > 0) {
        const summaries = session.results.slice(0, 5).map((r, i) => {
          return `${i + 1}. [${r.title}](${r.url})\n   Domain: ${r.domain}\n   Snippet: ${r.snippet || 'No snippet available'}`;
        });
        return `Search Results for "${q}":\n\n${summaries.join('\n\n')}`;
      }
      return session.answer || 'No search results found.';
    }

    if (name === 'web_open') {
      const url = args.url;
      try {
        const doc = await this.engine.fetchUrl(url);
        doc.id = `opened-${Date.now()}`;
        this.openedDocuments.set(url, doc);
        this.openedDocuments.set(doc.url, doc);

        const excerpt = doc.text.length > 1500 ? `${doc.text.slice(0, 1500)}...\n\n[Content truncated. Use web_find to locate specific information.]` : doc.text;
        return `Title: ${doc.title}\nDomain: ${doc.domain}\nURL: ${doc.url}\n\n${excerpt}`;
      } catch (err: any) {
        return `Error opening ${url}: ${err.message || String(err)}`;
      }
    }

    if (name === 'web_find') {
      const { url, query } = args;
      let doc = this.openedDocuments.get(url);
      if (!doc) {
        // If not already opened, open it first
        const openResult = await this.executeTool('web_open', { url });
        doc = this.openedDocuments.get(url);
        if (!doc) {
          return `Could not find "${query}" because page could not be opened: ${openResult}`;
        }
      }

      // Step 3: FIND -> Semantic/lexical retrieval inside the specific document
      const chunks = chunkDocument(doc, { targetTokens: 300, overlapTokens: 50 });
      if (chunks.length === 0) {
        return `Document at ${url} has no searchable text.`;
      }

      const bm25 = new InMemoryBM25();
      bm25.buildIndex(chunks);
      const hits = bm25.search(query, 2);

      if (hits.length > 0) {
        return hits
          .map((h, i) => {
            const heading = h.chunk.headingPath && h.chunk.headingPath.length > 0 ? `[Section: ${h.chunk.headingPath.join(' > ')}]\n` : '';
            return `Match ${i + 1}:\n${heading}${h.chunk.text}`;
          })
          .join('\n\n---\n\n');
      }

      // Fallback substring search
      const qLower = query.toLowerCase();
      const directMatch = chunks.find((c) => c.text.toLowerCase().includes(qLower));
      if (directMatch) {
        return `Match 1:\n${directMatch.text}`;
      }

      return `No matches found for "${query}" within ${url}.`;
    }

    throw new Error(`Unknown tool: ${name}`);
  }
}
