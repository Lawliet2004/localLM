import { describe, it, expect, afterEach, beforeEach, vi } from 'vitest';
import { installWebFixtures } from './test-fixtures';
import { WebAgentTools } from './tools';
import { WebSearchEngine } from './index';
import { MockSearchProvider } from './search/provider';
import type { SearchResult } from './types';

describe('Web Agent Tools Interface (Section 91 & 92)', () => {
  const originalFetch = globalThis.fetch;
  beforeEach(installWebFixtures);
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    globalThis.fetch = originalFetch;
  });

  it('exposes minimal, clean tool definitions to LLM', () => {
    const tools = new WebAgentTools();
    const defs = tools.getToolDefinitions();

    expect(defs.some((d) => d.name === 'web_search')).toBe(true);
    expect(defs.some((d) => d.name === 'weather')).toBe(true);
    expect(defs.some((d) => d.name === 'web_open')).toBe(true);
    expect(defs.some((d) => d.name === 'web_find')).toBe(true);

    const searchDef = defs.find((d) => d.name === 'web_search');
    expect(searchDef?.parameters).toBeDefined();
  });

  it('executes weather tool with structured results', async () => {
    const tools = new WebAgentTools();
    const output = await tools.executeTool('weather', { location: 'Ranaghat' });

    expect(output).toContain('Ranaghat');
    expect(output).toContain('Temperature');
  }, 15000);

  it('executes web_search and returns progressive disclosure summaries', async () => {
    const mockResults: SearchResult[] = [
      {
        id: 'r1',
        queryId: 'q',
        title: 'React 19 Release',
        url: 'https://react.dev/blog/2024/12/05/react-19',
        snippet: 'React 19 introduces server components and actions.',
        domain: 'react.dev',
        rank: 1,
      },
    ];
    const engine = new WebSearchEngine({ searchProvider: new MockSearchProvider({ 'react': mockResults }) });
    const tools = new WebAgentTools(engine);

    const output = await tools.executeTool('web_search', { query: 'react 19 release' });
    expect(output).toContain('Search Results');
    expect(output).toContain('React 19 Release');
    expect(output).toContain('react.dev');
  });

  it('executes web_open to read page and web_find to locate specific passages', async () => {
    globalThis.fetch = async (input: any) => {
      const url = String(input);
      if (url.includes('docs.example.com')) {
        return new Response(
          `<html>
            <body>
              <main>
                <h1>Documentation</h1>
                <p>The system context window is 128K tokens.</p>
                <h2>Performance</h2>
                <p>HumanEval score is 88.4 percent on verified coding benchmarks.</p>
              </main>
            </body>
          </html>`,
          { status: 200, headers: { 'Content-Type': 'text/html' } }
        );
      }
      return new Response('Not found', { status: 404 });
    };

    const tools = new WebAgentTools();
    const openRes = await tools.executeTool('web_open', { url: 'https://docs.example.com/specs' });
    expect(openRes).toContain('Title: Documentation');
    expect(openRes).toContain('128K tokens');

    const findRes = await tools.executeTool('web_find', {
      url: 'https://docs.example.com/specs',
      query: 'HumanEval score benchmark',
    });
    expect(findRes).toContain('HumanEval score is 88.4 percent');
  });
});
