import { describe, it, expect } from 'vitest';
import { assessSourceQuality } from './source_quality';

describe('Source Quality Assessment', () => {
  it('identifies official documentation sources and gives high authority score', () => {
    const res1 = assessSourceQuality('https://react.dev/reference/react/useActionState');
    expect(res1.sourceType).toBe('official_documentation');
    expect(res1.authorityScore).toBeGreaterThanOrEqual(0.9);
    expect(res1.isPrimary).toBe(true);

    const res2 = assessSourceQuality('https://docs.github.com/en/actions');
    expect(res2.sourceType).toBe('official_documentation');
  });

  it('identifies academic sources (arxiv, nature)', () => {
    const res = assessSourceQuality('https://arxiv.org/abs/2401.12345');
    expect(res.sourceType).toBe('academic_paper');
    expect(res.authorityScore).toBeGreaterThanOrEqual(0.9);
  });

  it('classifies forums and social media with appropriate lower priors', () => {
    const forum = assessSourceQuality('https://reddit.com/r/LocalLLaMA/comments/123');
    expect(forum.sourceType).toBe('community_forum');
    expect(forum.authorityScore).toBeLessThan(0.7);

    const social = assessSourceQuality('https://x.com/someone/status/123');
    expect(social.sourceType).toBe('social_media');
    expect(social.authorityScore).toBeLessThan(0.5);
  });
});
