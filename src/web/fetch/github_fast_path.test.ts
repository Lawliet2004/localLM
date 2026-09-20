import { expect, it } from 'vitest';
import { isUsableRawResponse, toGitHubRawCandidates } from './github_fast_path';

it('maps repository roots onto README raw URLs', () => {
  const candidates = toGitHubRawCandidates('https://github.com/deepseek-ai/DeepSeek-Coder-V2');
  expect(candidates?.map((c) => c.rawUrl)).toEqual([
    'https://raw.githubusercontent.com/deepseek-ai/DeepSeek-Coder-V2/HEAD/README.md',
    'https://raw.githubusercontent.com/deepseek-ai/DeepSeek-Coder-V2/main/README.md',
    'https://raw.githubusercontent.com/deepseek-ai/DeepSeek-Coder-V2/master/README.md',
  ]);
});

it('maps blob file views onto raw.githubusercontent.com', () => {
  const candidates = toGitHubRawCandidates('https://github.com/owner/repo/blob/main/src/index.ts');
  expect(candidates).toEqual([
    { rawUrl: 'https://raw.githubusercontent.com/owner/repo/main/src/index.ts', kind: 'blob' },
  ]);
});

it('ignores reserved github.com routes and non-github hosts', () => {
  expect(toGitHubRawCandidates('https://github.com/settings')).toBeNull();
  expect(toGitHubRawCandidates('https://github.com/owner/repo/issues/1')).toBeNull();
  expect(toGitHubRawCandidates('https://gitlab.com/owner/repo')).toBeNull();
});

it('rejects empty or HTML error bodies as unusable README content', () => {
  expect(isUsableRawResponse(200, 'text/plain', '404: Not Found', 'readme')).toBe(false);
  expect(isUsableRawResponse(200, 'text/markdown', '# DeepSeek Coder V2\n\nA long enough README body for the fast path.', 'readme')).toBe(true);
  expect(isUsableRawResponse(404, 'text/plain', 'missing', 'blob')).toBe(false);
});
