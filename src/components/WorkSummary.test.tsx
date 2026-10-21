import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it } from 'vitest';
import { WorkSummary } from './WorkSummary';
import type { Message, RunRecord } from '../lib/types';

const edit: Message = { id: 'edit', conversationId: 'chat', role: 'tool', status: 'complete', reasoning: '', createdAt: 1000, content: JSON.stringify({ request: { connector: 'Workspace', name: 'edit_file', arguments: { path: 'src/styles.css' }, decision: 'allowed' }, result: { diff: '--- a/src/styles.css\n+++ b/src/styles.css\n@@ -1 +1,2 @@\n-old\n+new\n+another\n' } }) };
const run: RunRecord = { id:'run', conversationId:'chat', status:'completed', createdAt:1000, updatedAt:232000 };

it('shows recorded duration and an expandable edit card with actual diff counts', async () => {
  render(<WorkSummary messages={[edit]} run={run}><p>Updated the styles.</p></WorkSummary>);
  expect(screen.getByText('Worked for 3m 51s')).toBeVisible();
  expect(screen.getByText('Edited 1 file')).toBeVisible();
  expect(screen.getAllByText('+2')[0]).toBeVisible();
  expect(screen.getAllByText('−1')[0]).toBeVisible();
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
  expect(screen.getByText('Edited 1 file')).toBeVisible();
  expect(screen.getAllByText('+4')).toHaveLength(2);
  await userEvent.click(screen.getByRole('button', { name: 'View changes' }));
  expect(screen.getAllByText(/--- a\/src\/styles.css/)).toHaveLength(2);
  expect(screen.getAllByText(/--- a\/src\/styles.css/)[0]).toBeVisible();
  await userEvent.click(screen.getByRole('button', { name: 'Collapse files' }));
  expect(screen.queryByText('styles.css')).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole('button', { name: 'Expand files' }));
  expect(screen.getByText('styles.css')).toBeVisible();
});

it('reads envelope diff data and excludes denied or failed envelope edits', () => {
  const content = (status: string) => JSON.stringify({ request: { name: 'edit_file', arguments: { path: 'note.txt' } }, result: { status, data: { diff: '--- a/note.txt\n+++ b/note.txt\n-old\n+new' } } });
  const view = render(<WorkSummary messages={[{ ...edit, content: content('success') }]}><p>Done.</p></WorkSummary>);
  expect(screen.getByText('Edited 1 file')).toBeVisible();
  view.rerender(<WorkSummary messages={[{ ...edit, content: content('failure') }]}><p>Failed.</p></WorkSummary>);
  expect(screen.queryByLabelText('File changes')).not.toBeInTheDocument();
});
