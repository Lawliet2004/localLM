import type { ComponentType, ReactNode } from 'react';
import {
  Archive,
  FileDiff,
  FileSearch,
  FolderSearch,
  Globe,
  ListChecks,
  Search,
  Sparkles,
  SquareTerminal,
  Wrench,
} from 'lucide-react';

/** Semantic category of an activity-trail entry. Drives icon + tint. */
export type TraceKind =
  | 'web'
  | 'edit'
  | 'task'
  | 'command'
  | 'search'
  | 'folder'
  | 'explore'
  | 'read'
  | 'tool'
  | 'thought'
  | 'system';

type IconComponent = ComponentType<{ size?: number | string; className?: string }>;

export const TRACE_ICONS: Record<TraceKind, IconComponent> = {
  web: Globe,
  edit: FileDiff,
  task: ListChecks,
  command: SquareTerminal,
  search: Search,
  folder: FolderSearch,
  explore: FolderSearch,
  read: FileSearch,
  tool: Wrench,
  thought: Sparkles,
  system: Archive,
};

export function TraceIcon({ kind, size = 13, children }: { kind: TraceKind; size?: number; children?: ReactNode }) {
  const Icon = TRACE_ICONS[kind] ?? Wrench;
  return (
    <i className={`trace-icon trace-icon-${kind}`} aria-hidden="true">
      {children ?? <Icon size={size} />}
    </i>
  );
}

interface ToolLikeArgs {
  [key: string]: unknown;
}

/**
 * Classify a tool invocation into a trace kind.
 * Precedence mirrors the verb logic in CommandRunCard so icon and label agree.
 */
export function toolKindOf(name: string, args: ToolLikeArgs = {}, result: ToolLikeArgs = {}): TraceKind {
  if (/web_search|web_open|web_find|web_fetch/.test(name)) return 'web';
  if (/edit|create_file|write_to_file|replace/.test(name) || typeof result.diff === 'string') return 'edit';
  if (name === 'manage_task') return 'task';
  const hasPath = Boolean(args.path ?? args.AbsolutePath ?? args.TargetFile ?? args.file);
  const isSearchName = /search|grep|find|list_files|locate/.test(name);
  if (/read_file|view_file|open_file|cat/.test(name) || (hasPath && !isSearchName)) return 'read';
  if (/list_dir|browse_dir/.test(name)) return 'folder';
  if (isSearchName) return 'search';
  if (args.command || args.CommandLine || args.code || /exec|run_code|terminal|run_command/.test(name)) return 'command';
  return 'tool';
}

/**
 * Pick a representative icon for a collapsed group of tool calls.
 * Homogeneous groups get their own icon; mixed exploration gets `explore`.
 */
export function groupKindOf(kinds: TraceKind[]): TraceKind {
  if (!kinds.length) return 'explore';
  const unique = new Set(kinds);
  if (unique.size === 1) return kinds[0];
  if (unique.has('web')) return 'web';
  if (unique.has('command')) return 'command';
  return 'explore';
}
