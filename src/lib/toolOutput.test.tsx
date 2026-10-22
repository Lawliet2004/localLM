import { act, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { ToolOutputBuffers, toolOutput, useToolOutput } from './toolOutput';

afterEach(() => vi.useRealTimers());
it('batches channel updates, isolates subscriptions, and exposes the final bytes synchronously', () => {
  vi.useFakeTimers();
  const buffers = new ToolOutputBuffers(), changed = vi.fn(), other = vi.fn();
  const unsubscribe = buffers.subscribe('chat', 'tool', changed);
  buffers.subscribe('another', 'tool', other);
  for (let i = 0; i < 100; i++) buffers.append('chat', 'tool', 'stdout', 'a');
  buffers.append('chat', 'tool', 'stderr', 'warning');
  expect(buffers.read('chat', 'tool')).toEqual({ stdout: 'a'.repeat(100), stderr: 'warning' });
  expect(changed).not.toHaveBeenCalled();
  vi.advanceTimersByTime(32);
  expect(changed).toHaveBeenCalledTimes(1);
  expect(other).not.toHaveBeenCalled();
  unsubscribe(); buffers.clear('chat', 'tool');
  expect(buffers.read('chat', 'tool')).toEqual({ stdout: '', stderr: '' });
});
it('rerenders only the subscribed tool, without a message-list update', () => {
  vi.useFakeTimers();
  const renders = { parent: 0, one: 0, two: 0 };
  function Tool({ id }: { id: 'one' | 'two' }) {
    renders[id]++;
    const output = useToolOutput('fixture', id);
    return <pre data-testid={id}>{output.stdout}</pre>;
  }
  function History() { renders.parent++; return <><Tool id="one" /><Tool id="two" /></>; }
  const view = render(<History />);
  const before = { ...renders };
  act(() => { for (let i = 0; i < 100; i++) toolOutput.append('fixture', 'one', 'stdout', 'x'); vi.advanceTimersByTime(32); });
  expect(screen.getByTestId('one')).toHaveTextContent('x'.repeat(100));
  expect(renders.parent).toBe(before.parent); expect(renders.two).toBe(before.two);
  expect(renders.one - before.one).toBe(1);
  view.unmount(); toolOutput.clear('fixture', 'one');
});
it('cancels a scheduled notification when a turn is disposed', () => {
  vi.useFakeTimers(); const buffers = new ToolOutputBuffers(), changed = vi.fn();
  buffers.subscribe('chat', 'tool', changed); buffers.append('chat', 'tool', 'stdout', 'partial');
  buffers.clear('chat', 'tool'); changed.mockClear(); vi.runAllTimers();
  expect(changed).not.toHaveBeenCalled();
});
