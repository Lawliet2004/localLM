import type { Message } from './types';
import { useEffect, useState } from 'react';
import { api, errorMessage } from './api';
import { toolStreamId } from './agentTranscript';

export function resultObject(value: unknown): Record<string, any> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, any> : {};
}

const records = new WeakMap<Message, Record<string, any>>();
const hydrated = new WeakMap<Message, { full: Record<string, any>; message: Message }>();
export function toolRecord(message: Message): Record<string, any> {
  const cached = records.get(message);
  if (cached) return cached;
  let record = {};
  try { record = resultObject(JSON.parse(message.content)); } catch { /* Legacy text. */ }
  records.set(message, record);
  return record;
}

export function toolResultData(result: unknown): Record<string, any> {
  const record = resultObject(result);
  return record.data && typeof record.data === 'object' && !Array.isArray(record.data) ? record.data : record;
}

export interface ToolOutcome {
  kind: 'running' | 'awaiting_approval' | 'success' | 'partial' | 'failure' | 'denied' | 'cancelled' | 'unknown';
  needsAttention: boolean;
}

/** One presentation interpretation of the dispatch envelope, with legacy
 * fallbacks. Never normalize or change the model-visible result here. */
export function toolOutcome(message: Message): ToolOutcome {
  const record = toolRecord(message), request = resultObject(record.request);
  const result = resultObject(record.result), data = toolResultData(result);
  let kind: ToolOutcome['kind'];
  if (request.decision === 'denied' || result.status === 'denied' || data.status === 'denied') kind = 'denied';
  else if (['partial', 'failure', 'cancelled', 'unknown'].includes(result.status)) kind = result.status;
  else if (['partial', 'cancelled', 'unknown'].includes(data.status)) kind = data.status;
  else if (result.status === 'interrupted' || data.status === 'interrupted') kind = 'unknown';
  else if (result.error || result.isError || data.error || data.isError || ['failed', 'failure', 'error', 'rolled_back'].includes(result.status) || ['failed', 'failure', 'error', 'rolled_back'].includes(data.status) || (typeof data.exitCode === 'number' && data.exitCode !== 0)) kind = 'failure';
  else if (message.status === 'interrupted') kind = 'unknown';
  else if (message.status === 'error') kind = 'failure';
  else if (request.decision === 'pending' || message.status === 'paused') kind = 'awaiting_approval';
  else if (message.status === 'streaming') kind = 'running';
  else kind = 'success';
  return { kind, needsAttention: kind !== 'success' };
}

export function toolRowKey(message: Message): string {
  const request = toolRecord(message).request;
  return request?.runId && request?.stepId && request?.callId
    ? toolStreamId({ conversationId: message.conversationId, runId: request.runId, stepId: request.stepId, toolCallId: request.callId })
    : message.id;
}

const mutationName = /^(edit_file|multi_edit|write_file|create_file|append_file|apply_patch|write_to_file|replace|move_file|delete_file)$/;
function isFileMutation(message: Message): boolean {
  return mutationName.test(String(toolRecord(message).request?.name ?? '').replace(/^workspace_/, ''));
}

/** Artifact storage is the authority when the model-visible result was bounded. */
export function useFullMutationResults(messages: Message[]): { messages: Message[]; error: string } {
  const [loaded, setLoaded] = useState<Record<string, Record<string, any>>>({});
  const [error, setError] = useState('');
  const key = JSON.stringify(messages.filter(isFileMutation).flatMap(message => {
    const id = toolRecord(message).result?._artifactId;
    return typeof id === 'string' ? [[id, message.conversationId]] : [];
  }));
  useEffect(() => {
    let disposed = false;
    setError('');
    const requests = new Map<string, string>(JSON.parse(key));
    void Promise.all([...requests].map(async ([id, conversationId]) => {
      try {
        const artifact = await api.getArtifact(id);
        if (!artifact || artifact.conversationId !== conversationId || artifact.mimeType !== 'application/json') throw new Error('Full edit result is unavailable in this conversation.');
        const result = resultObject(JSON.parse(artifact.content));
        if (!disposed) setLoaded(current => ({ ...current, [JSON.stringify([conversationId, id])]: result }));
      } catch (cause) { if (!disposed) setError(errorMessage(cause)); }
    }));
    return () => { disposed = true; };
  }, [key]);
  return { messages: messages.map(message => {
    const record = toolRecord(message);
    const full = loaded[JSON.stringify([message.conversationId, record.result?._artifactId])];
    if (!full || !isFileMutation(message)) return message;
    const cached = hydrated.get(message);
    if (cached?.full === full) return cached.message;
    const restored = { ...message, content: JSON.stringify({ ...record, result: full }) };
    hydrated.set(message, { full, message: restored });
    return restored;
  }), error };
}

export interface FileChange {
  path: string;
  diff?: string;
  artifactId?: string;
  kind?: string;
}

/** Read committed outcomes, never infer successful mutations from arguments alone. */
export function toolFileChanges(message: Message): FileChange[] {
  const record = toolRecord(message), request = resultObject(record.request);
  const result = resultObject(record.result), data = toolResultData(result);
  if (message.status !== 'complete' || request.decision === 'invalid' || toolOutcome(message).kind !== 'success') return [];
  const name = String(request.name ?? '').replace(/^workspace_/, '');
  if (!mutationName.test(name)) return [];
  const artifactId = typeof result._artifactId === 'string' ? result._artifactId : undefined;
  const changes = Array.isArray(data.changes) ? data.changes : [
    { ...data, path: data.path ?? request.arguments?.path },
  ];
  return changes.flatMap((raw: unknown) => {
    const change = resultObject(raw);
    if (typeof change.path !== 'string' || !change.path.trim()) return [];
    // A bounded result may retain paths without diffs. Show that limitation;
    // it is not evidence of zero changed lines.
    const diff = typeof change.diff === 'string' ? change.diff : undefined;
    if (!diff && !artifactId && data.bytesWritten === undefined && data.status !== 'committed' && !Array.isArray(data.changes)) return [];
    return [{ path: typeof change.destination === 'string' && change.destination.trim() ? change.destination : change.path, diff, artifactId, kind: change.kind }];
  });
}
