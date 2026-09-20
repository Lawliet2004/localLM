import { render, screen, fireEvent } from '@testing-library/react';
import { expect, it } from 'vitest';
import { PlanChecklist, parsePlanSteps, todosFromPlanEvents } from './PlanChecklist';
import type { SessionEvent, TodoItem } from '../lib/types';

const todos: TodoItem[] = [
  { text: 'Search fusion yield papers', status: 'completed', updatedAt: 1 },
  { text: 'Open the top source', status: 'in_progress', updatedAt: 2 },
  { text: 'Calculate the yield', status: 'pending', updatedAt: 3 },
];

it('shows progress, ticks completed work, and highlights the current task', () => {
  render(<PlanChecklist todos={todos} generating defaultOpen />);
  expect(screen.getByLabelText('Plan checklist')).toBeInTheDocument();
  expect(screen.getByText('1 of 3')).toBeInTheDocument();
  expect(screen.getByText('Search fusion yield papers')).toBeInTheDocument();
  expect(screen.getByText('Open the top source')).toBeInTheDocument();
  expect(screen.getByText('Now')).toBeInTheDocument();
  expect(screen.getByText('Search fusion yield papers').closest('li')).toHaveClass('completed');
  expect(screen.getByText('Open the top source').closest('li')).toHaveClass('in_progress');
});

it('collapses to a one-line preview of the current task', () => {
  render(<PlanChecklist todos={todos} defaultOpen={false} />);
  expect(screen.queryByRole('list')).not.toBeInTheDocument();
  expect(screen.getByText('Open the top source')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: /Plan/ }));
  expect(screen.getByRole('list')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: /Plan/ })).toHaveAttribute('aria-expanded', 'true');
});

it('parses numbered plan text into a checklist with the first item in progress', () => {
  const parsed = parsePlanSteps('# Plan\n\n1. **Search** papers\n2. Open the source\n3. Verify\n\nDone.');
  expect(parsed.map(item => item.text)).toEqual(['Search papers', 'Open the source', 'Verify']);
  expect(parsed[0].status).toBe('in_progress');
  expect(parsed[1].status).toBe('pending');
});

it('reads a fallback checklist from a plan_created session event', () => {
  const events: SessionEvent[] = [
    { id: '1', conversationId: 'c', seq: 1, eventType: 'plan_created', payload: { content: '1. Look up X\n2. Confirm Y' }, createdAt: 1, ignorable: false },
  ];
  expect(todosFromPlanEvents(events).map(item => item.text)).toEqual(['Look up X', 'Confirm Y']);
});
