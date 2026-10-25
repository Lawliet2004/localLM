import type { ComponentType, ReactNode } from 'react';

/** Semantic category of an activity-trail entry. Drives icon and row identity. */
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

/** Monochrome 24px line icons. Stroke follows `currentColor`, so rows own color. */
function line(paths: ReactNode): IconComponent {
  return function LineIcon({ size = 18, className }) {
    return (
      <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" className={className} aria-hidden="true" focusable="false">
        {paths}
      </svg>
    );
  };
}

const CommandGlyph = line(<><path d="M5 8l4 4-4 4" /><path d="M12 17h7" /></>);
const ReadGlyph = line(<><path d="M7 3.5h7l4 4V20.5H7z" /><path d="M14 3.5v4h4" /><path d="M10 12h5M10 15.5h5" /></>);
const EditGlyph = line(<><path d="M5 19l1-4L16.5 4.5a1.8 1.8 0 012.5 0l.5.5a1.8 1.8 0 010 2.5L9 18z" /><path d="M14.5 6.5l3 3" /><path d="M5 19l4-1" /></>);
const SearchGlyph = line(<><circle cx="11" cy="11" r="6" /><path d="M15.5 15.5L20 20" /></>);
const FolderGlyph = line(<><path d="M3.5 7.5v10a1 1 0 001 1h15a1 1 0 001-1v-8a1 1 0 00-1-1h-8l-2-2h-5a1 1 0 00-1 1z" /></>);
const WebGlyph = line(<><circle cx="12" cy="12" r="8.5" /><path d="M3.5 12h17" /><path d="M12 3.5c2.5 2.4 3.6 5.2 3.6 8.5s-1.1 6.1-3.6 8.5c-2.5-2.4-3.6-5.2-3.6-8.5S9.5 5.9 12 3.5z" /></>);
const TaskGlyph = line(<><path d="M5 7l1.6 1.6L9.5 5.7" /><path d="M12 7h7" /><path d="M5 15l1.6 1.6 2.9-2.9" /><path d="M12 15h7" /></>);
const ToolGlyph = line(<><path d="M14.5 6.5a4 4 0 004.9 4.9L11 19.8a2 2 0 01-2.8-2.8l8.4-8.4" /><path d="M14.5 6.5a4 4 0 014.9-1.4l-2.4 2.4 1 2.4" /></>);
const ThoughtGlyph = line(<><path d="M12 4v3M12 17v3M4 12h3M17 12h3" /><path d="M7 7l1.8 1.8M15.2 15.2L17 17M17 7l-1.8 1.8M8.8 15.2L7 17" /></>);
const SystemGlyph = line(<><rect x="4" y="5" width="16" height="4" rx="1" /><path d="M5.5 9v9.5h13V9" /><path d="M10 13h4" /></>);

export const TRACE_ICONS: Record<TraceKind, IconComponent> = {
  web: WebGlyph,
  edit: EditGlyph,
  task: TaskGlyph,
  command: CommandGlyph,
  search: SearchGlyph,
  folder: FolderGlyph,
  explore: FolderGlyph,
  read: ReadGlyph,
  tool: ToolGlyph,
  thought: ThoughtGlyph,
  system: SystemGlyph,
};

export function TraceIcon({ kind, size = 18, children }: { kind: TraceKind; size?: number; children?: ReactNode }) {
  const Icon = TRACE_ICONS[kind] ?? ToolGlyph;
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
  if (/apply_patch|edit|create_file|write_file|append_file|write_to_file|replace/.test(name) || typeof result.diff === 'string') return 'edit';
  if (name === 'manage_task' || name === 'plan_submit') return 'task';
  if (args.command || args.CommandLine || args.code || /exec|run_code|terminal|run_command/.test(name)) return 'command';
  const hasPath = Boolean(args.path ?? args.AbsolutePath ?? args.TargetFile ?? args.file);
  const isSearchName = /search|grep|find|list_files|list_tree|locate/.test(name);
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
