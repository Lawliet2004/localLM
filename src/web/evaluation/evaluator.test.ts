import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { installWebFixtures } from '../test-fixtures';
import { WebSearchEvaluator } from './evaluator';
import { EVALUATION_DATASET } from './dataset';
import { WebSearchEngine } from '../index';
import { MockSearchProvider } from '../search/provider';
import type { SearchResult } from '../types';

describe('Web Search & Grounding Evaluation Suite (52 Benchmark Questions)', () => {
  beforeEach(installWebFixtures);
  afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });
  it('contains at least 50 comprehensive benchmark questions', () => {
    expect(EVALUATION_DATASET.length).toBeGreaterThanOrEqual(50);
  });

  it('covers all 13 mandatory evaluation categories', () => {
    const categories = new Set(EVALUATION_DATASET.map((q) => q.category));
    expect(categories.has('weather')).toBe(true);
    expect(categories.has('recent_news')).toBe(true);
    expect(categories.has('software_version')).toBe(true);
    expect(categories.has('technical_documentation')).toBe(true);
    expect(categories.has('benchmark_comparison')).toBe(true);
    expect(categories.has('historical_fact')).toBe(true);
    expect(categories.has('multi_source_research')).toBe(true);
    expect(categories.has('ambiguous_query')).toBe(true);
    expect(categories.has('conflicting_sources')).toBe(true);
    expect(categories.has('missing_information')).toBe(true);
    expect(categories.has('bad_webpage')).toBe(true);
    expect(categories.has('duplicate_sources')).toBe(true);
    expect(categories.has('search_provider_failure')).toBe(true);
  });

  it('evaluates representative sample across categories with zero paid APIs', async () => {
    // Pick 1 question from each of the core categories for fast benchmark evaluation
    const sampleQuestions = [
      EVALUATION_DATASET.find((q) => q.id === 'W1')!, // weather
      EVALUATION_DATASET.find((q) => q.id === 'SV1')!, // software version
      EVALUATION_DATASET.find((q) => q.id === 'BC1')!, // benchmark comparison
      EVALUATION_DATASET.find((q) => q.id === 'HF1')!, // historical / static fact
      EVALUATION_DATASET.find((q) => q.id === 'MI1')!, // missing info
    ];

    const mockSearchResults: Record<string, SearchResult[]> = {
      'next.js': [
        {
          id: 's1',
          queryId: 'next',
          title: 'Next.js 15 Released',
          url: 'https://nextjs.org/blog/next-15',
          snippet: 'Next.js 15 is officially released with React 19 support and Turbopack dev.',
          domain: 'nextjs.org',
          rank: 1,
        },
      ],
      'qwen': [
        {
          id: 'q1',
          queryId: 'qwen',
          title: 'Qwen 2.5 Coder Benchmark',
          url: 'https://huggingface.co/Qwen/Qwen2.5-Coder-7B',
          snippet: 'Qwen 2.5 Coder achieves top scores on HumanEval.',
          domain: 'huggingface.co',
          rank: 1,
        },
      ],
      'deepseek': [
        {
          id: 'd1',
          queryId: 'deepseek',
          title: 'DeepSeek Coder V2',
          url: 'https://github.com/deepseek-ai/DeepSeek-Coder-V2',
          snippet: 'DeepSeek Coder benchmark metrics on coding evaluations.',
          domain: 'github.com',
          rank: 1,
        },
      ],
    };

    const engine = new WebSearchEngine({
      searchProvider: new MockSearchProvider(mockSearchResults),
    });

    const evaluator = new WebSearchEvaluator(engine);
    const scorecard = await evaluator.evaluateQuestions(sampleQuestions);

    expect(scorecard.evaluated).toBe(5);
    expect(scorecard.routingAccuracy).toBeGreaterThanOrEqual(0.8);
    expect(scorecard.citationCorrectnessRate).toBeGreaterThanOrEqual(0.9);
    expect(scorecard.averageContextTokens).toBeLessThan(4000);
  }, 30000);
});
