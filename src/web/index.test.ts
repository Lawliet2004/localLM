import {describe,it,expect,vi} from 'vitest';
import {WebSearchEngine,DEFAULT_CONFIG} from './index';
import {InMemoryStorageAdapter} from './cache/sqlite';
import type {SearchProvider,SearchResult} from './types';
const hit: SearchResult = {id:'one',queryId:'one',title:'Official source',url:'https://example.org/doc',domain:'example.org',snippet:'Unverified snippet',rank:1};
describe('Agent-controlled search and retrieval',()=>{
  it('performs the requested search once without nested model planning or synthesis',async()=>{
    const search=vi.fn(async()=>[hit]);
    const engine=new WebSearchEngine({searchProvider:{search}});
    const session=await engine.research('requested query');
    expect(search).toHaveBeenCalledTimes(1);
    expect(search.mock.calls[0]).toBeDefined();
    expect(session.queries.map(q=>q.query)).toEqual(['requested query']);
    expect(session.answer).toContain('Unverified snippet');
    expect(session.documents[0].metadata?.extractionMethod).toBe('snippet_only');
    expect(session.sources.S1.url).toBe(hit.url);
    expect(session.limitations?.join(' ')).toContain('unverified');
  });
  it('preserves filters, caches successful searches, and keeps failures visible',async()=>{
    const search=vi.fn<SearchProvider['search']>().mockResolvedValue([hit]);
    const engine=new WebSearchEngine({searchProvider:{search},storage:new InMemoryStorageAdapter()});
    await engine.research('same',{freshness:'day',language:'en',category:'science'});
    await engine.research('same',{freshness:'day',language:'en',category:'science'});
    expect(search).toHaveBeenCalledTimes(1);
    expect(search.mock.calls[0][0]).toMatchObject({query:'same',freshness:'day',language:'en',category:'science'});
    await engine.research('same',{freshness:'week'}); expect(search).toHaveBeenCalledTimes(2);
    search.mockRejectedValue(Object.assign(new Error('offline'),{retryable:false}));
    const result=await engine.research('other'); expect(result.trace?.searchFailureCount).toBe(1);
  });
  it('bounds search batches before provider calls',async()=>{
    const search=vi.fn(async()=>[]); const engine=new WebSearchEngine({searchProvider:{search}});
    await expect(engine.searchQueries(Array(5).fill('query'))).rejects.toThrow('1-4');
    await expect(engine.searchQueries([' '])).rejects.toThrow('nonempty');
    expect(search).not.toHaveBeenCalled();
  });
  it('deduplicates concurrent fetches of the same URL', async () => {
    let calls = 0;
    const engine = new WebSearchEngine({
      config: { cache: { ...DEFAULT_CONFIG.cache, enabled: false } },
      fetcher: { fetch: async url => {
        calls += 1;
        await new Promise((resolve) => setTimeout(resolve, 10));
        return { url, finalUrl: url, success: true, status: 200, durationMs: 0,
          body: `<html><article><h1>Shared</h1><p>${'Shared content for dedup testing. '.repeat(40)}</p></article></html>` };
      } },
    });
    const [a, b] = await Promise.all([
      engine.fetchUrl('https://example.com/shared'),
      engine.fetchUrl('https://example.com/shared'),
    ]);
    expect(calls).toBe(1);
    expect(a.text).toContain('Shared content');
    expect(b.text).toContain('Shared content');
  });

});
