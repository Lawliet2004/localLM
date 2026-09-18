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
  expect(screen.getByText('Edited styles.css')).toBeVisible();
  expect(screen.getByText('+2')).toBeVisible();
  expect(screen.getByText('−1')).toBeVisible();
  await userEvent.click(screen.getByText('Review'));
  expect(screen.getByText(/--- a\/src\/styles.css/)).toBeVisible();
});
it('does not present failed edits as successful changes or invent a duration', () => {
  render(<WorkSummary messages={[{...edit,status:'error'}]}><p>Editing failed.</p></WorkSummary>);
  expect(screen.queryByText('Edited styles.css')).not.toBeInTheDocument();
  expect(screen.queryByText(/Worked for/)).not.toBeInTheDocument();
  expect(screen.getByText('Work details')).toBeVisible();
});
it('does not call a cancelled run completed', () => {
  render(<WorkSummary messages={[]} run={{...run,status:'cancelled'}}><p>Stopped.</p></WorkSummary>);
  expect(screen.getByText('Stopped for 3m 51s')).toBeVisible();
});
