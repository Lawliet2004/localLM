import { expect, it } from 'vitest';
import type { Message } from './types';
import { toolFileChanges, toolOutcome } from './toolResult';

function tool(result: unknown, decision = 'allowed', status: Message['status'] = 'complete'): Message {
  return { id: 'tool', conversationId: 'chat', role: 'tool', reasoning: '', createdAt: 1, status,
    content: JSON.stringify({ request: { name: 'workspace_create_file', decision, arguments: { path: 'file.ts' } }, result }) };
}

it.each(['success', 'partial', 'failure', 'cancelled', 'unknown'] as const)('interprets the %s envelope consistently', kind => {
  const message = tool({ schema: 'locallm.tool-envelope/1', status: kind, data: { path: 'file.ts', bytesWritten: 12 } });
  expect(toolOutcome(message)).toEqual({ kind, needsAttention: kind !== 'success' });
  expect(toolFileChanges(message)).toHaveLength(kind === 'success' ? 1 : 0);
});

it('keeps denied, pending, interrupted and legacy nested failures consequential', () => {
  expect(toolOutcome(tool({}, 'denied')).kind).toBe('denied');
  expect(toolOutcome(tool({}, 'pending', 'streaming')).kind).toBe('awaiting_approval');
  expect(toolOutcome(tool({}, 'allowed', 'interrupted')).kind).toBe('unknown');
  expect(toolOutcome(tool({ data: { status: 'failure' } })).kind).toBe('failure');
  expect(toolOutcome(tool({ exitCode: 1 })).kind).toBe('failure');
  expect(toolOutcome(tool({}, 'allowed', 'streaming')).kind).toBe('running');
});
