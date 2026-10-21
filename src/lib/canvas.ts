// Extracts canvas-worthy artifacts (OpenUI programs, HTML, code blocks) from
// assistant messages. Blocks sharing a title are iterations of one artifact:
// later occurrences supersede earlier ones and form its version history.
import type { Message } from './types';

export type CanvasKind = 'openui' | 'html' | 'code';

export interface CanvasArtifact {
  /** Unique per fenced block: `${messageId}:${index}`. */
  id: string;
  /** Iterations of one artifact share this key: `${kind}:${normalized title}`. */
  groupKey: string;
  title: string;
  kind: CanvasKind;
  language: string;
  code: string;
  messageId: string;
  /** 1-based position within its group, in message order. */
  version: number;
}

const FENCE = /```([^\s`]*)\n([\s\S]*?)(?:```|$)/g;
const HEADING = /^#{1,6}\s+(.+?)\s*#*\s*$/gm;
const BOLD_LINE = /^\*\*(.+?)\*\*\s*$/gm;

function kindFor(language: string): CanvasKind {
  if (language === 'openui' || language === 'openui-lang') return 'openui';
  if (language === 'html' || language === 'svg') return 'html';
  return 'code';
}

function lastTitle(before: string): string {
  let title = '';
  let position = -1;
  for (const pattern of [HEADING, BOLD_LINE]) {
    pattern.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = pattern.exec(before))) {
      if (match.index >= position && match[1].trim()) { position = match.index; title = match[1].trim(); }
    }
  }
  return title.replace(/[*_`]/g, '').trim();
}

export function extractArtifacts(messages: Message[]): CanvasArtifact[] {
  const artifacts: CanvasArtifact[] = [];
  const counts = new Map<string, number>();
  for (const message of messages) {
    if (message.role !== 'assistant') continue;
    FENCE.lastIndex = 0;
    let match: RegExpExecArray | null;
    let index = 0;
    while ((match = FENCE.exec(message.content))) {
      const language = (match[1] || '').toLowerCase();
      const code = match[2].replace(/\n$/, '');
      if (!code.trim()) { index++; continue; }
      const kind = kindFor(language);
      const title = lastTitle(message.content.slice(0, match.index))
        || (kind === 'openui' ? 'Generated UI' : kind === 'html' ? 'HTML artifact' : `${language || 'code'} snippet`);
      const groupKey = `${kind}:${title.toLowerCase()}`;
      const version = (counts.get(groupKey) ?? 0) + 1;
      counts.set(groupKey, version);
      artifacts.push({ id: `${message.id}:${index}`, groupKey, title, kind, language, code, messageId: message.id, version });
      index++;
    }
  }
  return artifacts;
}

/** Versions of one artifact group, oldest first. */
export function artifactVersions(artifacts: CanvasArtifact[], groupKey: string): CanvasArtifact[] {
  return artifacts.filter(artifact => artifact.groupKey === groupKey);
}

/** Find the artifact a chat block should open, by exact code and language. */
export function findArtifact(artifacts: CanvasArtifact[], code: string, language: string): CanvasArtifact | undefined {
  const normalized = code.replace(/\n$/, '');
  return artifacts.find(artifact => artifact.code === normalized && artifact.language === language.toLowerCase());
}
