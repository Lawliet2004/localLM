import { describe, it, expect } from 'vitest';
import { deduplicateSearchResults, tokenJaccardSimilarity, levenshteinSimilarity } from './deduplicator';
import type { SearchResult } from '../types';

describe('Result Deduplicator', () => {
  it('computes Token Jaccard similarity', () => {
    const sim = tokenJaccardSimilarity(
      'Model X Released with 9B parameters',
      'Model X launched with 9B parameters'
    );
    expect(sim).toBeGreaterThan(0.6);
  });

  it('computes Levenshtein similarity', () => {
    const sim = levenshteinSimilarity('hello world', 'hello world');
    expect(sim).toBe(1.0);
  });

  it('deduplicates identical normalized URLs and merges query IDs', () => {
    const results: SearchResult[] = [
      {
        id: 'r1',
        queryId: 'Q1',
        title: 'React 19 Release',
        url: 'https://react.dev/blog/2024/12/05/react-19?utm_source=rss',
        domain: 'react.dev',
        rank: 1,
      },
      {
        id: 'r2',
        queryId: 'Q2',
        title: 'React 19 Release',
        url: 'https://www.react.dev/blog/2024/12/05/react-19',
        domain: 'react.dev',
        rank: 2,
      },
    ];

    const deduped = deduplicateSearchResults(results);
    expect(deduped).toHaveLength(1);
    expect(deduped[0].queryId).toContain('Q1');
    expect(deduped[0].queryId).toContain('Q2');
  });

  it('deduplicates same-domain near-identical syndicated titles', () => {
    const results: SearchResult[] = [
      {
        id: 'r1',
        queryId: 'Q1',
        title: 'SpaceX Starship Completes Fourth Flight Test Successfully',
        url: 'https://news.com/article-1',
        domain: 'news.com',
        rank: 1,
      },
      {
        id: 'r2',
        queryId: 'Q2',
        title: 'SpaceX Starship Completes Fourth Flight Test Successfully Today',
        url: 'https://news.com/article-mirror-2',
        domain: 'news.com',
        rank: 2,
      },
    ];

    const deduped = deduplicateSearchResults(results);
    expect(deduped).toHaveLength(1);
  });

  it('deduplicates cross-domain syndicated wire articles (Section 19: Reuters/AP)', () => {
    const results: SearchResult[] = [
      {
        id: 'r1',
        queryId: 'Q1',
        title: 'Federal Reserve cuts interest rates by 25 basis points',
        url: 'https://reuters.com/business/fed-rate-cut',
        domain: 'reuters.com',
        rank: 1,
      },
      {
        id: 'r2',
        queryId: 'Q2',
        title: 'Federal Reserve cuts interest rates by 25 basis points',
        url: 'https://finance.yahoo.com/news/fed-rate-cut-syndicated',
        domain: 'finance.yahoo.com',
        rank: 2,
      },
    ];

    const deduped = deduplicateSearchResults(results);
    expect(deduped).toHaveLength(1);
    expect(deduped[0].domain).toBe('reuters.com');
  });
});
