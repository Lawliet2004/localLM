import type { ComponentType, ReactNode } from 'react';
import {
  Archive,
  Compass,
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

import type { ToolKind } from '../lib/toolActivity';
export { toolKindOf } from '../lib/toolActivity';
export type TraceKind = ToolKind;

type IconComponent = ComponentType<{ size?: number | string; className?: string }>;

export const TRACE_ICONS: Record<TraceKind, IconComponent> = {
  web: Globe,
  edit: FileDiff,
  task: ListChecks,
  command: SquareTerminal,
  search: Search,
  folder: FolderSearch,
  explore: Compass,
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
