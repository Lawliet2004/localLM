import { safeSourceUrl, sourceIdentity } from '../lib/sources';
import { ExternalLink } from 'lucide-react';
import { api } from '../lib/api';
import { useEffect, useMemo, useState } from 'react';
import type { Message, SessionEvent } from '../lib/types';

export interface AnswerSource { url: string; title?: string; snippet?: string; date?: string; ids: string[] }
export function collectAnswerSources(messages: Message[], events: SessionEvent[] = []): AnswerSource[] {
  const sources = new Map<string, AnswerSource>();
  function visit(value: unknown, depth = 0) {
    if (!value || typeof value !== 'object' || depth > 6) return;
    const object = value as Record<string, unknown>;
    if (object.sources && typeof object.sources === 'object') {
      const entries = Array.isArray(object.sources) ? object.sources.map((source, index) => [String(index + 1), source] as const) : Object.entries(object.sources);
      for (const [id, raw] of entries) {
        const source = typeof raw === 'string' ? { url: raw } : raw as Record<string, unknown> | null;
        if (!source || typeof source.url !== 'string') continue;
        try {
          const url = safeSourceUrl(source.url);
          if (!url) continue;
          const key = sourceIdentity(url);
          const existing = sources.get(key);
          const sid = String(source.id ?? source.sid ?? id);
          const text = (key: string) => typeof source[key] === 'string' ? source[key] as string : undefined;
          sources.set(key, { url: source.url, title: text('title') ?? text('name') ?? existing?.title, snippet: text('snippet') ?? existing?.snippet, date: text('publishedAt') ?? text('date') ?? existing?.date, ids: [...new Set([...(existing?.ids ?? []), sid])] });
        } catch { /* Invalid stored link is not an actionable source. */ }
      }
    }
    for (const [key, nested] of Object.entries(object)) if (key !== 'sources') visit(nested, depth + 1);
  }
  for (const message of messages) if (message.role === 'tool') { try { visit(JSON.parse(message.content)); } catch { /* Legacy text. */ } }
  for (const event of events) if (event.eventType === 'tool_result') visit(event.payload);
  return [...sources.values()];
}
export interface AnswerEvidence { sources: AnswerSource[]; metadataError: boolean }
const artifactCache = new Map<string, Promise<Message>>();
function loadSourceArtifact(id: string, conversationId: string): Promise<Message> {
  const key = JSON.stringify([conversationId, id]);
  const cached = artifactCache.get(key);
  if (cached) return cached;
  const request = api.getArtifact(id).then(artifact => {
    if (!artifact || artifact.conversationId !== conversationId || artifact.mimeType !== 'application/json') throw Error('Unavailable source metadata');
    JSON.parse(artifact.content);
    return { id, conversationId, role: 'tool' as const, status: 'complete' as const, content: artifact.content, reasoning: '', createdAt: 0 };
  }).catch(error => { artifactCache.delete(key); throw error; });
  artifactCache.set(key, request);
  if (artifactCache.size > 64) artifactCache.delete(artifactCache.keys().next().value!);
  return request;
}

/** Inline citations and the source list consume this same hydrated evidence. */
export function useAnswerEvidence(messages: Message[], events: SessionEvent[] = [], enabled = true): AnswerEvidence {
  const [full, setFull] = useState<Message[]>([]);
  const [metadataError, setMetadataError] = useState(false);
  const conversationId = messages[0]?.conversationId ?? '';
  const artifactKey = useMemo(() => {
    const ids = new Set<string>();
    const visit = (value: unknown, depth = 0) => {
      if (!value || typeof value !== 'object' || depth > 6) return;
      const record = value as Record<string, unknown>;
      if (typeof record._artifactId === 'string') ids.add(record._artifactId);
      for (const child of Object.values(record)) if (child && typeof child === 'object') visit(child, depth + 1);
    };
    for (const message of messages) if (message.role === 'tool') {
      try { const record = JSON.parse(message.content); if (/web|search|fetch|visit|research/.test(String(record.request?.name ?? record.name ?? ''))) visit(record.result); } catch { /* Legacy text. */ }
    }
    for (const event of events) if (event.eventType === 'tool_result' && /web|search|fetch|visit|research/.test(String((event.payload as { name?: string })?.name ?? ''))) visit(event.payload);
    return JSON.stringify([...ids].sort());
  }, [messages, events]);
  useEffect(() => {
    setFull([]); setMetadataError(false);
    const ids: string[] = JSON.parse(artifactKey);
    if (!enabled || !ids.length || !api.getArtifact) return;
    let disposed = false;
    void Promise.allSettled(ids.slice(0, 64).map(id => loadSourceArtifact(id, conversationId))).then(values => {
      if (disposed) return;
      setFull(values.flatMap(value => value.status === 'fulfilled' ? [value.value] : []));
      setMetadataError(ids.length > 64 || values.some(value => value.status === 'rejected'));
    });
    return () => { disposed = true; };
  }, [artifactKey, conversationId, enabled]);
  const sources = useMemo(() => collectAnswerSources([...messages, ...full.filter(message => message.conversationId === conversationId)], events), [messages, full, events, conversationId]);
  return { sources, metadataError };
}

/** Ambiguous source IDs deliberately remain unlinked rather than linking to
 * whichever tool happened to finish last. Full source URLs stay actionable. */
export function answerSourceLinks(sources: AnswerSource[]): string {
  const links = new Map<string, string | null>();
  for (const source of sources) for (const id of source.ids) {
    const prior = links.get(id);
    links.set(id, prior !== undefined && prior !== source.url ? null : source.url);
  }
  return JSON.stringify(Object.fromEntries([...links].filter((entry): entry is [string, string] => entry[1] !== null)));
}
function hostOf(url: string): string {
  try { return new URL(url).hostname; } catch { return url; }
}

export function AnswerSources({ messages, events = [], evidence }: { messages: Message[]; events?: SessionEvent[]; evidence?: AnswerEvidence }) {
  const recovered = useAnswerEvidence(messages, events, !evidence);
  const { sources, metadataError } = evidence ?? recovered;
  if (!sources.length && !metadataError) return null;
  // Citation IDs stay available on hover; the visible line names the domain only.
  return <section className="answer-sources" aria-label="Answer sources"><h3>Sources <span>{sources.length}</span></h3><ol className="web-source-list">{sources.map((source, index) => <li key={source.url} className="web-source"><span className="web-source-index" aria-hidden="true">{index + 1}</span><span className="web-source-main"><a className="web-source-link" href={source.url} target="_blank" rel="noreferrer">{source.title || hostOf(source.url)}</a><small className="web-source-domain" title={source.ids.length ? `Citation ${source.ids.join(', ')}` : undefined}>{hostOf(source.url)}{source.date ? ` · ${source.date}` : ''}</small>{source.snippet && <p className="answer-source-snippet">{source.snippet}</p>}</span><ExternalLink size={12} className="web-source-open" aria-hidden="true" /></li>)}</ol>{metadataError && <p role="status">Some stored source details are unavailable. Existing source links remain available.</p>}</section>;
}
