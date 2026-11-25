import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Chat } from './Chat';
import type { Message } from '../lib/types';

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: {
    listInstalledModels: async () => [],
    compactionStatus: async () => ({ auto: true, keepLast: 10, checkpoint: null }),
    getArtifact: vi.fn(async () => null),
    contextPreflight: async () => ({ available: false }),
    compactConversation: async () => ({ note: '', checkpoint: null }),
    getTodos: async () => [],
  },
}));

const props = { generating: false, ready: true, loading: false, onSend: vi.fn(), onCancel: vi.fn(), onConfigure: vi.fn() };

const base: Message = { id: '', conversationId: 'c1', role: 'tool', content: '', reasoning: '', status: 'complete', createdAt: 0 };
const tool = (id: string, record: unknown, createdAt: number): Message => ({ ...base, id, content: JSON.stringify(record), createdAt });

/** Deterministic mixed conversation: prose, reasoning, ok+failed commands,
    code, search, file read, diff, artifact capture, and an unknown tool. */
const fixture: Message[] = [
  { ...base, id: 'u1', role: 'user', content: 'Fix the flaky test and explain.', createdAt: 1 },
  tool('t-search', {
    request: { connector: 'Web', name: 'web_search', arguments: { question: 'vitest fake timers' } },
    result: { status: 'success', answer: 'Use vi.useFakeTimers.', sources: { S1: { title: 'Vitest docs', url: 'https://vitest.dev/guide/' } } },
  }, 2),
  tool('t-read', {
    request: { connector: 'Workspace', name: 'read_file', arguments: { path: 'src/timer.ts', start_line: 1, line_count: 5 } },
    result: { status: 'success', content: '1: export function tick() {\n2:   return 1;\n3: }', path: 'src/timer.ts', totalLines: 3, sha256: 'ab12' },
  }, 3),
  tool('t-cmd-ok', {
    request: { connector: 'Shell', name: 'execute_command', arguments: { command: 'npm test', cwd: '/repo' } },
    result: { status: 'success', data: { logs: { stdout: '12 passed', stderr: 'npm warn old dep' }, exitCode: 0 }, durationMs: 800 },
  }, 4),
  tool('t-cmd-fail', {
    request: { connector: 'Shell', name: 'execute_command', arguments: { command: 'cargo build' } },
    result: { status: 'error', isError: true, data: { logs: { stderr: 'error[E0308]: mismatched types' }, exitCode: 101 }, error: { message: 'build failed' } },
  }, 5),
  tool('t-edit', {
    request: { connector: 'Workspace', name: 'edit_file', arguments: { path: 'src/timer.ts' } },
    result: { status: 'success', diff: '--- a/src/timer.ts\n+++ b/src/timer.ts\n@@ -1,3 +1,3 @@\n export function tick() {\n-  return 1;\n+  return Date.now();\n }' },
  }, 6),
  tool('t-artifact', {
    request: { connector: 'Shell', name: 'execute_command', arguments: { command: 'cat huge.log' } },
    result: { status: 'success', _truncated: true, _artifactId: 'art_' + 'a'.repeat(32), _originalBytes: 51200, data: { logs: { stdout: 'first lines…' }, exitCode: 0 } },
  }, 7),
  tool('t-unknown', {
    request: { connector: 'MCP · Research', name: 'acme_telemetry_probe', arguments: { depth: 3 } },
    result: { status: 'success', isError: false },
  }, 8),
  {
    ...base, id: 'a1', role: 'assistant', createdAt: 9,
    reasoning: 'The test flakes because `tick()` returns a constant.',
    content: 'Found it — `tick()` returns a constant. Here is the fix:\n\n```ts\nexport function tick() {\n  return Date.now();\n}\n```\n\nThe build failure is unrelated (E0308).',
  },
];

describe('mixed conversation fixture', () => {
  it('keeps a failed command envelope out of terminal output when it has no logs', () => {
    const timeout = tool('t-timeout', {
      request: { connector: 'Harness', name: 'shell_run', arguments: { command: 'npm run check' } },
      result: { status: 'failure', isError: true, error: { code: 'timeout', message: 'The command reached its time limit.' } },
    }, 1);
    render(<Chat {...props} messages={[timeout]} />);
    expect(screen.getByRole('alert')).toHaveTextContent('The command reached its time limit.');
    expect(screen.queryByRole('button', { name: 'Output' })).not.toBeInTheDocument();
    expect(screen.getByText('Execution details')).toBeVisible();
  });

  it('shows a failed check report even when the recorded process has no logs', () => {
    const report = tool('t-check', {
      request: { connector: 'Harness', name: 'check_report', arguments: { id: 'process-1' } },
      result: {
        status: 'failure', isError: true,
        error: { code: 'checks-command_failed', message: 'The command exited unsuccessfully.' },
        data: {
          processId: 'process-1', process: { id: 'process-1', state: 'exited', exitCode: 101 },
          report: { verdict: 'command_failed', cases: [{ name: 'test_parse' }] },
        },
      },
    }, 1);
    render(<Chat {...props} messages={[report]} />);
    expect(screen.getByText('Check report')).toBeVisible();
    expect(screen.getByText('The command exited unsuccessfully.')).toBeVisible();
    expect(screen.getByTestId('result-viewport')).toHaveTextContent('test_parse');
    expect(screen.queryByRole('button', { name: 'Output' })).not.toBeInTheDocument();
  });

  it('renders every activity category with its distinct surface', () => {
    render(<Chat {...props} messages={fixture} />);

    // Prose + generated code with real highlighting.
    expect(screen.getByText(/Found it —/)).toBeInTheDocument();
    const codeBlocks = document.querySelectorAll('.message-assistant code.hljs');
    expect(codeBlocks.length).toBeGreaterThan(0);
    expect(codeBlocks[0].querySelector('.hljs-keyword')).toHaveTextContent('export');

    // One stable reasoning disclosure for the turn.
    const reasoning = screen.getAllByTestId('turn-reasoning');
    expect(reasoning).toHaveLength(1);
    expect(reasoning[0]).not.toHaveTextContent('constant');
    fireEvent.click(reasoning[0].querySelector('summary')!);
    expect(reasoning[0]).toHaveTextContent('constant');

    // Search card with validated citation links.
    expect(screen.getByText('WebSearch')).toBeInTheDocument();
    const links = screen.getAllByRole('link', { name: 'Vitest docs' });
    expect(links[0]).toHaveAttribute('href', 'https://vitest.dev/guide/');

    // Successful command keeps exit status separate from its stderr channel.
    expect(screen.getAllByTestId('status-exit-0').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('stderr').length).toBeGreaterThanOrEqual(1);

    // Failed command is red-flagged with exit code and error.
    expect(screen.getByTestId('status-exit-101')).toBeInTheDocument();
    expect(screen.getByText('error[E0308]: mismatched types')).toBeInTheDocument();

    // Consequential commands and edits remain individually inspectable.
    expect(document.querySelector('.command-run-card[data-kind=command]')?.closest('.activity-summary')).toBeNull();
    expect(document.querySelector('.command-run-card[data-kind=edit]')?.closest('.activity-summary')).toBeNull();
  });

  it('surfaces file reads, diffs, artifacts and unknown tools deterministically', () => {
    render(<Chat {...props} messages={fixture} />);

    // File read renders through the dedicated path card.
    expect(document.querySelector('.file-result > summary')).toHaveAccessibleName('Read file src/timer.ts (lines 1–3)');
    expect(screen.getByText('Read 3 lines')).toBeInTheDocument();
    expect(screen.getAllByText('src/timer.ts').length).toBeGreaterThanOrEqual(1);

    // Diff counts appear on the work summary and the edit card header.
    expect(screen.getAllByText('+1').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('−1').length).toBeGreaterThanOrEqual(1);

    for (const row of document.querySelectorAll<HTMLDetailsElement>('.command-run-card')) if (!row.open) { const title = row.querySelector('summary')!; const event = new MouseEvent('click', { bubbles: true, cancelable: true }); fireEvent(title, event); }

    // Artifact-captured output exposes the artifact identity and size.
    expect(screen.getByText(/art_a{32}/)).toBeInTheDocument();
    expect(screen.getByText('(50.0 KB)')).toBeInTheDocument();

    // Unknown tool falls back to a labelled generic card, not raw JSON.
    expect(screen.getByText(/acme_telemetry_probe/)).toBeInTheDocument();
  });

  it('renders an interrupted tool with a neutral label', () => {
    const messages: Message[] = [
      { ...base, id: 'u1', role: 'user', content: 'run it', createdAt: 1 },
      { ...tool('t-int', { request: { connector: 'Shell', name: 'execute_command', arguments: { command: 'sleep 99' } }, result: {} }, 2), status: 'interrupted' },
      { ...base, id: 'a1', role: 'assistant', content: 'Stopped.', createdAt: 3 },
    ];
    render(<Chat {...props} messages={messages} />);
    expect(screen.getByTestId('status-interrupted')).toBeInTheDocument();
  });

  it('renders no reasoning disclosure when the model supplied none', () => {
    const messages: Message[] = [
      { ...base, id: 'u1', role: 'user', content: 'Hi', createdAt: 1 },
      { ...base, id: 'a1', role: 'assistant', content: 'Plain answer, no reasoning field.', createdAt: 2 },
    ];
    render(<Chat {...props} messages={messages} />);
    expect(screen.queryByTestId('turn-reasoning')).toBeNull();
    expect(screen.getByText('Plain answer, no reasoning field.')).toBeInTheDocument();
  });

  it('degrades gracefully on malformed and legacy tool payloads', () => {
    const messages: Message[] = [
      { ...base, id: 'u1', role: 'user', content: 'go', createdAt: 1 },
      // Legacy history: content is plain text, not a JSON request/result record.
      { ...base, id: 't-legacy', content: 'Ran legacy_tool: ok', createdAt: 2 },
      // Malformed record: JSON parses but has no request shape.
      tool('t-malformed', { unexpected: 'shape' }, 3),
      { ...base, id: 'a1', role: 'assistant', content: 'Done.', createdAt: 4 },
    ];
    render(<Chat {...props} messages={messages} />);
    expect(screen.getAllByTestId('command-run-card').length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText('Done.')).toBeInTheDocument();
  });

  it('keeps a streaming answer readable with an unfinished code fence', () => {
    const messages: Message[] = [
      { ...base, id: 'u1', role: 'user', content: 'show code', createdAt: 1 },
      { ...base, id: 'a1', role: 'assistant', status: 'streaming', createdAt: 2,
        content: 'Here is the snippet:\n\n```ts\nconst x = 1;\nconst y' },
    ];
    render(<Chat {...props} messages={messages} generating />);
    // The unclosed fence still renders as a code block with partial text.
    const code = document.querySelector('.message-assistant code.hljs');
    expect(code).toHaveTextContent('const x = 1;');
    expect(code).toHaveTextContent('const y');
  });
});
