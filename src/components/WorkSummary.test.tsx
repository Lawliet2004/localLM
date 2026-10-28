import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { api } from '../lib/api';
import { WorkSummary } from './WorkSummary';
import type { Message, RunRecord } from '../lib/types';

const edit: Message = { id: 'edit', conversationId: 'chat', role: 'tool', status: 'complete', reasoning: '', createdAt: 1000, content: JSON.stringify({ request: { connector: 'Workspace', name: 'edit_file', arguments: { path: 'src/styles.css' }, decision: 'allowed' }, result: { diff: '--- a/src/styles.css\n+++ b/src/styles.css\n@@ -1 +1,2 @@\n-old\n+new\n+another\n' } }) };
const run: RunRecord = { id:'run', conversationId:'chat', status:'completed', createdAt:1000, updatedAt:232000 };

it('shows recorded duration and an expandable edit card with actual diff counts', async () => {
  render(<WorkSummary messages={[edit]} run={run}><p>Updated the styles.</p></WorkSummary>);
  expect(screen.getByText('Worked for 3m 51s')).toBeVisible();
  expect(screen.getByText('1 file changed')).toBeVisible();
  expect(screen.getAllByText('+2')[0]).toBeVisible();
  expect(screen.getAllByText('−1')[0]).toBeVisible();
  await userEvent.click(screen.getByRole('button', { name: /1 file changed/ }));
  await userEvent.click(screen.getByRole('button', { name: 'View changes' }));
  expect(screen.getByText(/--- a\/src\/styles.css/)).toBeVisible();
});
it('does not present failed edits as successful changes or invent a duration', () => {
  render(<WorkSummary messages={[{...edit,status:'error'}]}><p>Editing failed.</p></WorkSummary>);
  expect(screen.queryByLabelText('File changes')).not.toBeInTheDocument();
  expect(screen.queryByText(/Worked for/)).not.toBeInTheDocument();
  expect(screen.getByText('Work details')).toBeVisible();
});
it('does not call a cancelled run completed', () => {
  render(<WorkSummary messages={[]} run={{...run,status:'cancelled'}}><p>Stopped.</p></WorkSummary>);
  expect(screen.getByText('Stopped for 3m 51s')).toBeVisible();
});

it('groups repeated edits by path and supports viewing and collapsing files', async () => {
  render(<WorkSummary messages={[edit, { ...edit, id: 'second-edit' }]}><p>Done.</p></WorkSummary>);
  expect(screen.getByText('1 file changed')).toBeVisible();
  expect(screen.getAllByText('+4')).toHaveLength(1);
  await userEvent.click(screen.getByRole('button', { name: /1 file changed/ }));
  await userEvent.click(screen.getByRole('button', { name: 'View changes' }));
  expect(screen.getAllByText(/--- a\/src\/styles.css/)).toHaveLength(2);
  expect(screen.getAllByText(/--- a\/src\/styles.css/)[0]).toBeVisible();
  await userEvent.click(screen.getByRole('button', { name: /1 file changed/ }));
  expect(screen.queryByText('styles.css')).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: /1 file changed/ }));
  expect(screen.getByText('styles.css')).toBeVisible();
});

it('reads envelope diff data and excludes denied or failed envelope edits', () => {
  const content = (status: string) => JSON.stringify({ request: { name: 'edit_file', arguments: { path: 'note.txt' } }, result: { status, data: { diff: '--- a/note.txt\n+++ b/note.txt\n-old\n+new' } } });
  const view = render(<WorkSummary messages={[{ ...edit, content: content('success') }]}><p>Done.</p></WorkSummary>);
  expect(screen.getByText('1 file changed')).toBeVisible();
  view.rerender(<WorkSummary messages={[{ ...edit, content: content('failure') }]}><p>Failed.</p></WorkSummary>);
  expect(screen.queryByLabelText('File changes')).not.toBeInTheDocument();
});

it('renders the native multi-file transaction contract instead of requiring a top-level path', async () => {
  const changes = ['a.ts', 'b.ts'].map(path => ({ path, kind: 'edit', diff: `--- a/${path}\n+++ b/${path}\n@@ -1 +1 @@\n-old\n+new\n` }));
  const message = { ...edit, content: JSON.stringify({ request: { name: 'workspace_apply_patch', arguments: { changes: [{ path: 'a.ts' }, { path: 'b.ts' }] } }, result: { schemaVersion: 'locallm.tool-envelope/1', status: 'success', data: { status: 'committed', changes } } }) };
  render(<WorkSummary messages={[message]}><p>Done.</p></WorkSummary>);
  await userEvent.click(screen.getByRole('button', { name: /2 files changed/ }));
  expect(screen.getByText('a.ts')).toBeVisible();
  expect(screen.getByText('b.ts')).toBeVisible();
});

it('shows actual file creation without inventing zero line counts', async () => {
  const message = { ...edit, content: JSON.stringify({ request: { name: 'create_file', arguments: { path: 'new.ts' } }, result: { status: 'success', data: { path: 'new.ts', bytesWritten: 30 } } }) };
  render(<WorkSummary messages={[message]}><p>Created.</p></WorkSummary>);
  await userEvent.click(screen.getByRole('button', { name: /1 file changed/ }));
  expect(screen.getByText('new.ts')).toBeVisible();
  expect(screen.queryByText('+0')).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'View changes' }));
  expect(screen.getByText(/line totals are unavailable/)).toBeVisible();
});

it('restores artifact-bounded transaction diffs from conversation-owned storage', async () => {
  const full = { status: 'success', data: { status: 'committed', changes: [{ path: 'restored.ts', diff: '--- a/restored.ts\n+++ b/restored.ts\n-old\n+new' }] } };
  const spy = vi.spyOn(api, 'getArtifact').mockResolvedValue({ id: 'art_test', conversationId: 'chat', toolName: 'apply_patch', mimeType: 'application/json', sizeBytes: 100, sha256: 'hash', createdAt: 1, content: JSON.stringify(full) });
  try {
    const bounded = { ...edit, content: JSON.stringify({ request: { name: 'workspace_apply_patch', arguments: { changes: [] } }, result: { status: 'partial', _artifactId: 'art_test', data: {} } }) };
    render(<WorkSummary messages={[bounded]}><p>Done.</p></WorkSummary>);
    await waitFor(() => expect(screen.getByText('1 file changed')).toBeInTheDocument());
    await userEvent.click(screen.getByRole('button', { name: /1 file changed/ }));
    expect(screen.getByText('restored.ts')).toBeVisible();
  } finally { spy.mockRestore(); }
});

it('never reports rolled-back or uncertain native changes as successful files', () => {
  const messages = ['rolled_back', 'unknown'].map(status => ({ ...edit, id: status, content: JSON.stringify({ request: { name: 'apply_patch' }, result: { status: 'success', data: { status, changes: [{ path: 'not-committed.ts', diff: '-old\n+new' }] } } }) }));
  render(<WorkSummary messages={messages}><p>Failed.</p></WorkSummary>);
  expect(screen.queryByLabelText('File changes')).not.toBeInTheDocument();
});

it('keeps pending verification visible after a completed assistant answer', () => {
  const event = { id: 'v', conversationId: 'chat', runId: 'run', seq: 20, eventType: 'verification_status', createdAt: 1000, ignorable: false, payload: { status: 'pending', message: 'Checks are still running; tests have not been verified.' } };
  render(<WorkSummary messages={[]} run={run} events={[event]}><p>Answer complete.</p></WorkSummary>);
  expect(screen.getByRole('status')).toHaveTextContent('Checks are still running; tests have not been verified.');
});

it('rejects full mutation artifacts from another conversation', async () => {
  const spy = vi.spyOn(api, 'getArtifact').mockResolvedValue({ id: 'art_other', conversationId: 'foreign', toolName: 'apply_patch', mimeType: 'application/json', sizeBytes: 100, sha256: 'hash', createdAt: 1, content: JSON.stringify({ status: 'committed', changes: [{ path: 'foreign.ts', diff: '-old\n+new' }] }) });
  try {
    const bounded = { ...edit, content: JSON.stringify({ request: { name: 'workspace_apply_patch' }, result: { status: 'partial', _artifactId: 'art_other', data: {} } }) };
    render(<WorkSummary messages={[bounded]}><p>Done.</p></WorkSummary>);
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('unavailable in this conversation'));
    expect(screen.queryByLabelText('File changes')).not.toBeInTheDocument();
  } finally { spy.mockRestore(); }
});
