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
export function AnswerSources({ messages, events = [] }: { messages: Message[]; events?: SessionEvent[] }) {
  const [full, setFull] = useState<Message[]>([]);
  const [metadataError, setMetadataError] = useState(false);
  const conversationId = messages[0]?.conversationId;
  const artifactKey = JSON.stringify([...new Set(messages.filter(message => message.role === 'tool').flatMap(message => {
    try { const value = JSON.parse(message.content); return /web|search|fetch|visit|research/.test(String(value.request?.name ?? value.name ?? '')) && typeof value.result?._artifactId === 'string' ? [value.result._artifactId as string] : []; } catch { return []; }
  }))].sort());
  useEffect(() => {
    setFull([]); setMetadataError(false);
    const ids: string[] = JSON.parse(artifactKey);
    if (!ids.length || !api.getArtifact) return;
    let disposed = false;
    void Promise.all(ids.slice(0, 64).map(async id => {
      try {
        const artifact = await api.getArtifact(id);
        if (!artifact || artifact.conversationId !== conversationId || artifact.mimeType !== 'application/json') throw Error('Unavailable source metadata');
        JSON.parse(artifact.content);
        return { id, conversationId: conversationId ?? '', role: 'tool' as const, status: 'complete' as const, content: artifact.content, reasoning: '', createdAt: 0 };
      } catch { if (!disposed) setMetadataError(true); return null; }
    })).then(values => { if (!disposed) setFull(values.filter(value => value !== null)); });
    return () => { disposed = true; };
  }, [artifactKey, conversationId]);
  const sources = useMemo(() => collectAnswerSources([...messages, ...full.filter(message => message.conversationId === conversationId)], events), [messages, full, events, conversationId]);
  if (!sources.length) return null;
  return <section className="answer-sources" aria-label="Answer sources"><h3>Sources <span>{sources.length}</span></h3><ol className="web-source-list">{sources.map((source, index) => <li key={source.url} className="web-source"><span className="web-source-index" aria-hidden="true">{index + 1}</span><span className="web-source-main"><a className="web-source-link" href={source.url} target="_blank" rel="noreferrer">{source.title || new URL(source.url).hostname}</a><small className="web-source-domain">{new URL(source.url).hostname} · {source.ids.map(id => `[${id}]`).join(' ')}{source.date ? ` · ${source.date}` : ''}</small>{source.snippet && <p className="answer-source-snippet">{source.snippet}</p>}</span><ExternalLink size={12} className="web-source-open" aria-hidden="true" /></li>)}</ol>{metadataError && <p>Some stored source details are unavailable. Existing source links remain available.</p>}</section>;
}
