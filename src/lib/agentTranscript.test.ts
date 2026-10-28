import { expect, it } from 'vitest';
import { appendModelDelta, toolStreamId, withLiveRounds } from './agentTranscript';
import type { ChatEvent, Message, SessionEvent } from './types';

const message: Message = { id: 'answer', conversationId: 'chat', role: 'assistant', content: '', reasoning: '', status: 'streaming', createdAt: 1 };
const delta = (stepId: string, seq: number, content = '', reasoning = ''): ChatEvent => ({ messageId: 'answer', runId: 'run', stepId, seq, content, reasoning });
const event = (seq: number, stepId: string, eventType: string, payload: unknown = {}): SessionEvent => ({ id: String(seq), conversationId: 'chat', runId: 'run', stepId, seq, eventType, payload, createdAt: seq, ignorable: false });

it('scopes provider call IDs to the conversation, run and step without ambiguous delimiters', () => {
  const identity = { conversationId: 'chat', runId: 'run', stepId: 'one', toolCallId: 'same' };
  expect(new Set([identity, { ...identity, stepId: 'two' }, { ...identity, runId: 'other' }, { ...identity, conversationId: 'other' }].map(toolStreamId)).size).toBe(4);
  expect(toolStreamId({ conversationId: 'chat', toolCallId: 'old' })).toBe('tool-old');
});

it('retains separate round snapshots and ignores repeated sequenced deltas', () => {
  const one = appendModelDelta(message, delta('one', 3, 'Inspect.', 'Read first.'));
  expect(appendModelDelta(one, delta('one', 3, 'Inspect.', 'Read first.'))).toBe(one);
  const two = appendModelDelta(one, delta('two', 10, 'Fixed.', 'Verify.'));
  expect(two.content).toBe('Inspect.Fixed.');
  expect(two.liveRounds?.map(round => [round.stepId, round.content, round.reasoning])).toEqual([['one', 'Inspect.', 'Read first.'], ['two', 'Fixed.', 'Verify.']]);
  expect(appendModelDelta(two, delta('two', 11))).toBe(two);
});

it('places live rounds inside journal steps and replaces overlays with committed rows', () => {
  const live = appendModelDelta(appendModelDelta(message, delta('one', 100, 'Inspect.', 'Read.')), delta('two', 200, 'Fixed.', 'Verify.'));
  const journal = [event(5, 'one', 'step_start'), event(8, 'one', 'tool_call'), event(9, 'one', 'tool_result'), event(12, 'two', 'step_start')];
  const projected = withLiveRounds(journal, live).sort((a, b) => a.seq - b.seq);
  expect(projected.map(row => row.eventType)).toEqual(['step_start', 'reasoning', 'model_response', 'tool_call', 'tool_result', 'step_start', 'reasoning', 'model_response']);
  const committed = [...journal, event(6, 'one', 'reasoning', { text: 'Read.' }), event(7, 'one', 'model_response', { content: 'Inspect.', final: false })];
  expect(withLiveRounds(committed, live).filter(row => row.stepId === 'one' && row.eventType === 'reasoning')).toEqual([committed[4]]);
  expect(withLiveRounds(journal, { ...live, conversationId: 'different' })).toEqual(journal);
});

it('preserves legacy unscoped deltas without inventing model rounds', () => {
  const result = appendModelDelta(message, { messageId: 'answer', content: 'Legacy', reasoning: '' });
  expect(result.content).toBe('Legacy');
  expect(result.liveRounds).toBeUndefined();
});

it('removes only a rejected provider attempt while retaining earlier committed work',()=>{
  const live=appendModelDelta(appendModelDelta(message,delta('one',1,'Committed.','First.')),delta('two',2,'Unfinished','Partial'));
  const recovered=appendModelDelta(live,{...delta('two',0),discardStep:'two'});
  expect(recovered.content).toBe('Committed.'); expect(recovered.reasoning).toBe('First.');
  expect(recovered.liveRounds?.map(round=>round.stepId)).toEqual(['one']);
});
