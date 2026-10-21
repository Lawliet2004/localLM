import { describe, expect, it } from 'vitest';
import { artifactVersions, extractArtifacts, findArtifact } from './canvas';
import type { Message } from './types';

function message(id: string, content: string, role: 'assistant' | 'user' = 'assistant'): Message {
  return { id, conversationId: 'c', role, content, reasoning: '', status: 'complete', createdAt: 0 };
}

describe('extractArtifacts', () => {
  it('collects openui, html and code blocks from assistant messages only', () => {
    const artifacts = extractArtifacts([
      message('u1', '```openui\nroot = Response([])\n```', 'user'),
      message('a1', 'Text\n\n```openui\nroot = Response([])\n```\n\n```html\n<p>x</p>\n```\n\n```python\nprint(1)\n```'),
    ]);
    expect(artifacts.map(a => [a.kind, a.language])).toEqual([
      ['openui', 'openui'], ['html', 'html'], ['code', 'python'],
    ]);
    expect(artifacts.map(a => a.title)).toEqual(['Generated UI', 'HTML artifact', 'python snippet']);
  });

  it('groups iterations of one artifact by title and numbers the versions', () => {
    const artifacts = extractArtifacts([
      message('a1', '## Dashboard\n\n```openui\nroot = Response([s])\ns = Stat("A", "1")\n```'),
      message('a2', 'Here is a revision.\n\n## Dashboard\n\n```openui\nroot = Response([s])\ns = Stat("A", "2")\n```'),
    ]);
    expect(artifacts).toHaveLength(2);
    expect(artifacts[0].groupKey).toBe(artifacts[1].groupKey);
    expect(artifacts.map(a => a.version)).toEqual([1, 2]);
    expect(artifactVersions(artifacts, artifacts[0].groupKey)).toHaveLength(2);
  });

  it('does not group same-titled blocks of different kinds', () => {
    const artifacts = extractArtifacts([
      message('a1', '**Widget**\n\n```html\n<p>a</p>\n```'),
      message('a2', '**Widget**\n\n```js\nlet a = 1;\n```'),
    ]);
    expect(artifacts[0].groupKey).not.toBe(artifacts[1].groupKey);
    expect(artifacts.map(a => a.version)).toEqual([1, 1]);
  });

  it('finds a block by its exact code and language', () => {
    const artifacts = extractArtifacts([message('a1', '```python\nprint(1)\n```')]);
    expect(findArtifact(artifacts, 'print(1)', 'python')?.id).toBe('a1:0');
    expect(findArtifact(artifacts, 'print(2)', 'python')).toBeUndefined();
  });
});
