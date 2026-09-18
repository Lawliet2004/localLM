import { describe, it, expect } from 'vitest';
import { normalizeRequest } from './request_normalizer';

describe('Request Normalizer (Section 8)', () => {
  it('detects location and enriches weather queries while preserving intent', () => {
    const res = normalizeRequest('weather today in ranaghat');
    expect(res.originalQuery).toBe('weather today in ranaghat');
    expect(res.normalizedQuery).toBe('weather today in Ranaghat, West Bengal, India');
    expect(res.language).toBe('en');
    expect(res.questionType).toBe('weather');
    expect(res.entities.some((e) => e.type === 'location' && e.value.includes('Ranaghat'))).toBe(true);
    expect(res.dateExpressions).toContain('today');
  });

  it('detects software, version, and quoted terms in queries', () => {
    const res = normalizeRequest('What changed in "React 19" recently?');
    expect(res.originalQuery).toBe('What changed in "React 19" recently?');
    expect(res.questionType).toBe('software');
    expect(res.quotedTerms).toContain('React 19');
    expect(res.dateExpressions).toContain('recently');
    expect(res.entities.some((e) => e.type === 'software' && e.value === 'react')).toBe(true);
  });

  it('detects comparison questions and model entities', () => {
    const res = normalizeRequest('Compare current coding benchmarks for Qwen and DeepSeek');
    expect(res.questionType).toBe('comparison');
    expect(res.entities.some((e) => e.type === 'model' && e.value === 'qwen')).toBe(true);
    expect(res.entities.some((e) => e.type === 'model' && e.value === 'deepseek')).toBe(true);
    expect(res.dateExpressions).toContain('current');
  });

  it('preserves URLs present in queries', () => {
    const res = normalizeRequest('Summarize https://react.dev/blog/2024/12/05/react-19');
    expect(res.urls).toContain('https://react.dev/blog/2024/12/05/react-19');
    expect(res.entities.some((e) => e.type === 'url')).toBe(true);
  });
});
