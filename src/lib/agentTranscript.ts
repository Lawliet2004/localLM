import type { ChatEvent, Message, SessionEvent, ToolIdentity } from './types';

/** Provider call IDs are only unique within a model step. Legacy transports
 * without scope keep their old identity until persisted history is loaded. */
export function toolStreamId(identity: ToolIdentity): string {
  return identity.runId && identity.stepId
    ? `tool-${JSON.stringify([identity.conversationId, identity.runId, identity.stepId, identity.toolCallId])}`
    : `tool-${identity.toolCallId}`;
}

export function appendModelDelta(message: Message, event: ChatEvent): Message {
  if (event.discardStep) {
    const discarded = (message.liveRounds ?? []).filter(round => round.runId === event.runId && round.stepId === event.discardStep);
    const removeSuffix = (text: string, suffix: string) => suffix && text.endsWith(suffix) ? text.slice(0, -suffix.length) : text;
    return { ...message, content:removeSuffix(message.content, discarded.map(round=>round.content).join('')),
      reasoning:removeSuffix(message.reasoning, discarded.map(round=>round.reasoning).join('')),
      liveRounds:(message.liveRounds ?? []).filter(round => !(round.runId === event.runId && round.stepId === event.discardStep)) };
  }
  if (!event.content && !event.reasoning) return message;
  if (!event.runId || !event.stepId) return { ...message, content: message.content + event.content, reasoning: message.reasoning + event.reasoning };
  const rounds = message.liveRounds ?? [];
  const index = rounds.findIndex(round => round.runId === event.runId && round.stepId === event.stepId);
  const previous = rounds[index];
  // Sequence zero is channel-only progress and is never a delta identity.
  if (event.seq && previous && event.seq <= previous.lastSeq) return message;
  const round = {
    conversationId: message.conversationId, runId: event.runId, stepId: event.stepId,
    roundIndex: event.roundIndex ?? previous?.roundIndex ?? rounds.length,
    content: (previous?.content ?? '') + event.content,
    reasoning: (previous?.reasoning ?? '') + event.reasoning,
    lastSeq: event.seq ?? previous?.lastSeq ?? 0,
    createdAt: previous?.createdAt ?? Date.now(),
  };
  return { ...message, content: message.content + event.content, reasoning: message.reasoning + event.reasoning,
    liveRounds: index < 0 ? [...rounds, round] : rounds.map((value, i) => i === index ? round : value) };
}

/** Overlay unfinished rounds on the committed journal. Local channel sequence
 * numbers and journal sequence numbers are different clocks: place each overlay
 * inside its recorded step, rather than comparing the two counters. */
export function withLiveRounds(events: SessionEvent[], message?: Message): SessionEvent[] {
  const rounds = message?.liveRounds;
  if (!message || !rounds?.length) return events;
  const result = [...events];
  let cursor = 0;
  for (const [index, round] of rounds.entries()) {
    if (round.conversationId !== message.conversationId) continue;
    const step = events.filter(event => event.runId === round.runId && event.stepId === round.stepId);
    const start = step.find(event => event.eventType === 'step_start');
    const anchor = start?.seq ?? (step.length ? Math.min(...step.map(event => event.seq)) - 0.5 : cursor + 1);
    const add = (eventType: string, payload: Record<string, unknown>, offset: number) => {
      if (step.some(event => event.eventType === eventType)) return;
      result.push({ id: `live-${JSON.stringify([round.runId, round.stepId, eventType])}`, conversationId: round.conversationId,
        runId: round.runId, stepId: round.stepId, seq: anchor + offset, eventType, payload,
        createdAt: round.createdAt, ignorable: true });
    };
    if (round.reasoning) add('reasoning', { text: round.reasoning, live: message.status === 'streaming' && index === rounds.length - 1 && !step.some(event => event.eventType === 'tool_call') }, 0.1);
    if (round.content) add('model_response', { content: round.content, final: index === rounds.length - 1 && !step.some(event => event.eventType === 'tool_call') }, 0.2);
    cursor = Math.max(cursor, anchor + 0.2, ...step.map(event => event.seq));
  }
  return result;
}
