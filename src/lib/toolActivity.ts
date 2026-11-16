import type { Message } from './types';

export type ToolKind = 'web' | 'edit' | 'task' | 'command' | 'search' | 'folder' | 'explore' | 'read' | 'tool' | 'thought' | 'system';
const records = new WeakMap<Message, Record<string, any>>();
/** Parse an immutable message once for every transcript consumer. */
export function toolRecord(message: Message): Record<string, any> {
  const cached = records.get(message);
  if (cached) return cached;
  let record: Record<string, any> = {};
  try { const value = JSON.parse(message.content); if (value && typeof value === 'object' && !Array.isArray(value)) record = value; } catch { /* Legacy text. */ }
  records.set(message, record);
  return record;
}
export function toolOperationId(callId: string, runId?: string | null, stepId?: string | null): string {
  return runId || stepId ? `tool-${JSON.stringify([runId ?? null, stepId ?? null, callId])}` : `tool-${callId}`;
}
const web = new Set(['search', 'visit', 'web_search', 'web_open', 'web_find', 'web_fetch', 'web_fetch_url', 'get_weather', 'weather']);
const edits = new Set(['file_write', 'write_file', 'create_file', 'append_file', 'edit_file', 'edit_lines', 'apply_patch', 'multi_edit', 'write_file_base64', 'copy_file', 'move_file', 'delete_file', 'create_directory', 'delete_directory', 'copy_directory', 'move_directory', 'notebook_edit', 'recover_patch', 'format_paths', 'lsp_rename', 'lsp_apply_edit', 'lsp_execute_command']);
const commands = new Set(['shell_run', 'run_code', 'execute_command', 'local_run_code', 'local_execute_command', 'process_start', 'process_poll', 'process_write', 'process_stop', 'check_run', 'check_report', 'verification_run', 'verification_report', 'terminal_send', 'terminal_read', 'terminal_open', 'terminal_close']);
const reads = new Set(['file_read', 'read_file', 'file_info', 'notebook_read', 'document_inspect', 'document_read', 'view_image', 'view_file', 'open_file']);
const searches = new Set(['file_search', 'search_content', 'glob_files', 'search_files', 'grep', 'find', 'lsp_symbols', 'lsp_references']);
/** Explicit native aliases take precedence over generic argument hints. */
export function toolKindOf(name: string, args: Record<string, unknown> = {}, result: Record<string, unknown> = {}): ToolKind {
  const native = name.replace(/^workspace_/, '');
  if (web.has(name)) return 'web';
  if (edits.has(native) || typeof result.diff === 'string' || Array.isArray(result.changes)) return 'edit';
  if (['manage_task', 'plan_submit', 'todo_write', 'todo_read', 'subagent', 'workflow_run'].includes(name)) return 'task';
  if (commands.has(name)) return 'command';
  if (reads.has(native)) return 'read';
  if (['list_files', 'list_tree', 'list_directory', 'list_dir', 'browse_dir', 'project_inspect'].includes(native)) return 'folder';
  if (searches.has(native)) return 'search';
  if (args.command || args.CommandLine || args.code) return 'command';
  // Unknown connector tools retain a neutral label rather than being declared
  // a read merely because they accept a path.
  return 'tool';
}
export function toolFailed(record: Record<string, any>): boolean {
  const result = record.result ?? {};
  return Boolean(result.error || result.isError || ['error', 'failed', 'failure', 'denied', 'interrupted', 'cancelled', 'unknown'].includes(result.status) || record.request?.decision === 'denied');
}
export interface ChangedFile { path: string; destination?: string; kind: string; beforeSha256?: string; sha256?: string; diff?: string }
export function changedFiles(message: Message): ChangedFile[] {
  const record = toolRecord(message), request = record.request ?? record, result = record.result ?? {}, data = result.data ?? result;
  if (message.status !== 'complete' || toolFailed(record) || toolKindOf(request.name ?? '', request.arguments, data) !== 'edit') return [];
  const raw = Array.isArray(data.changes) ? data.changes : [{ ...data, path: data.path ?? request.arguments?.path }];
  return raw.flatMap((change: Record<string, unknown>) => typeof change.path === 'string' ? [{
    path: change.path, destination: typeof change.destination === 'string' ? change.destination : undefined,
    kind: typeof change.kind === 'string' ? change.kind : /create|file_write/.test(request.name ?? '') ? 'write' : 'edit',
    beforeSha256: typeof change.beforeSha256 === 'string' ? change.beforeSha256 : undefined,
    sha256: typeof change.sha256 === 'string' ? change.sha256 : undefined,
    diff: typeof change.diff === 'string' ? change.diff : undefined,
  }] : []);
}
