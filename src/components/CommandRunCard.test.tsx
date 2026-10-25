import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { CommandRunCard } from './CommandRunCard';
import { api } from '../lib/api';

vi.mock('../lib/api', () => ({
  api: {
    getArtifact: vi.fn(),
  },
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

it('renders web search as a compact searched-the-web card with sources', () => {
  render(
    <CommandRunCard
      toolName="web_search"
      status="completed"
      arguments={{ question: 'fusion yield 2024' }}
      structuredResult={JSON.stringify({
        answer: 'Laser fusion experiments reported a scientific yield above one.',
        sources: {
          S1: { title: 'NIF result', url: 'https://example.com/nif' },
          S2: { title: 'Nature paper', url: 'https://example.com/nature' },
        },
      })}
    />
  );
  expect(screen.getByText('Searched the web')).toBeInTheDocument();
  expect(screen.getByText('fusion yield 2024')).toBeInTheDocument();
  expect(screen.getByText('2 sources')).toBeInTheDocument();
  fireEvent.click(screen.getByText('fusion yield 2024'));
  expect(screen.getByRole('link', { name: 'NIF result' })).toHaveAttribute('href', 'https://example.com/nif');
  expect(document.querySelector('.web-answer-excerpt')).toHaveTextContent('scientific yield above one');
});

it('renders running state with pulse indicator and running badge', () => {
  render(
    <CommandRunCard
      toolName="execute_command"
      command="cargo test"
      status="running"
      stdout="Compiling test..."
    />
  );
  expect(screen.getByTestId('status-running')).toBeInTheDocument();
  expect(screen.getByText('Running…')).toBeInTheDocument();
  expect(screen.getByText('cargo test')).toBeInTheDocument();
  expect(screen.getByText('Compiling test...')).toBeInTheDocument();
});

it('renders exit 0 badge on successful command execution', () => {
  render(
    <CommandRunCard
      toolName="execute_command"
      command="npm test"
      status="completed"
      exitCode={0}
      stdout="All tests passed"
      durationMs={1250}
    />
  );
  expect(screen.getByTestId('status-exit-0')).toBeInTheDocument();
  expect(screen.getByText('Exit 0')).toBeInTheDocument();
  expect(screen.getByText('1.3s')).toBeInTheDocument();
});

it('renders exit 1 badge on failed command execution', () => {
  render(
    <CommandRunCard
      toolName="execute_command"
      command="python script.py"
      status="failed"
      exitCode={1}
      stderr="Traceback (most recent call last)"
    />
  );
  expect(screen.getByTestId('status-exit-1')).toBeInTheDocument();
  expect(screen.getByText('Exit 1')).toBeInTheDocument();
});

it('parses ANSI colors and formats styled spans in terminal viewport', () => {
  // \x1b[32m is green, \x1b[31m is red, \x1b[0m is reset
  const ansiText = '\x1b[32mPASS\x1b[0m test_parser\n\x1b[31mFAIL\x1b[0m test_stream';
  render(
    <CommandRunCard
      toolName="execute_command"
      command="test-runner"
      status="running"
      stdout={ansiText}
    />
  );
  const passSpan = screen.getByText('PASS');
  expect(passSpan).toBeInTheDocument();
  expect(passSpan).toHaveStyle({ color: 'var(--terminal-green, #10b981)' });

  const failSpan = screen.getByText('FAIL');
  expect(failSpan).toBeInTheDocument();
  expect(failSpan).toHaveStyle({ color: 'var(--terminal-red, #ef4444)' });
});

it('renders unified diff view with add and delete lines highlighted', async () => {
  const diff = '--- a/file.txt\n+++ b/file.txt\n@@ -1,2 +1,2 @@\n-old line\n+new line';
  render(
    <CommandRunCard
      toolName="edit_file"
      environment="Workspace"
      status="running"
      diff={diff}
    />
  );
  expect(screen.getByTestId('diff-viewport')).toBeInTheDocument();
  expect(screen.getByTestId('diff-viewport').querySelector('.diff-line.del .diff-content')).toHaveTextContent('-old line');
  expect(screen.getByTestId('diff-viewport').querySelector('.diff-line.add .diff-content')).toHaveTextContent('+new line');
});

it('supports folding long outputs exceeding 30 lines', () => {
  const longOutput = Array.from({ length: 45 }, (_, i) => `log line ${i + 1}`).join('\n');
  render(
    <CommandRunCard
      toolName="execute_command"
      command="dump-logs"
      status="completed"
      stdout={longOutput}
    />
  );
  fireEvent.click(screen.getByTestId('command-run-card').querySelector('summary')!);
  expect(screen.getByText('Show all 45 lines')).toBeInTheDocument();
  expect(screen.getByText('log line 1')).toBeInTheDocument();
  expect(screen.queryByText('log line 35')).not.toBeInTheDocument();

  fireEvent.click(screen.getByText('Show all 45 lines'));
  expect(screen.getByText('Show fewer lines')).toBeInTheDocument();
  expect(screen.getByText('log line 35')).toBeInTheDocument();
});

it('copies combined output to clipboard', async () => {
  const writeTextMock = vi.fn().mockResolvedValue(undefined);
  Object.assign(navigator, {
    clipboard: {
      writeText: writeTextMock,
    },
  });

  render(
    <CommandRunCard
      toolName="execute_command"
      command="echo hello"
      status="running"
      stdout="hello world"
    />
  );
  const copyBtn = screen.getByTitle('Copy terminal output');
  fireEvent.click(copyBtn);
  expect(writeTextMock).toHaveBeenCalledWith('hello world');
  expect(await screen.findByText('Copied')).toBeInTheDocument();
});

it('inspects bounded artifact content on demand', async () => {
  vi.mocked(api.getArtifact).mockResolvedValueOnce({
    id: 'art-cmd-1',
    conversationId: 'chat',
    runId: 'run-1',
    mimeType: 'text/plain',
    sizeBytes: 2048,
    toolName: 'execute_command',
    sha256: 'xyz',
    content: 'Full unabridged terminal output',
    createdAt: 0,
  });

  render(
    <CommandRunCard
      toolName="execute_command"
      command="long-command"
      status="completed"
      artifactId="art-cmd-1"
      originalBytes={2048}
    />
  );

  fireEvent.click(screen.getByTestId('command-run-card').querySelector('summary')!);
  expect(screen.getByText(/Full result captured in artifact/)).toBeInTheDocument();
  expect(screen.getByText('art-cmd-1')).toBeInTheDocument();
  expect(screen.getByText('(2.0 KB)')).toBeInTheDocument();

  const inspectBtn = screen.getByRole('button', { name: 'Inspect artifact' });
  fireEvent.click(inspectBtn);

  await waitFor(() => expect(api.getArtifact).toHaveBeenCalledWith('art-cmd-1'));
  expect(await screen.findByText('Full unabridged terminal output')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Loaded' })).toBeInTheDocument();
});

it('renders code tab and switches to inspect source code', () => {
  render(
    <CommandRunCard
      toolName="run_code"
      language="python"
      code={"print('running python snippet')\nexit(0)"}
      status="completed"
      exitCode={0}
      stdout="running python snippet"
    />
  );
  fireEvent.click(screen.getByTestId('command-run-card').querySelector('summary')!);
  const codeTab = screen.getByRole('button', { name: 'Code' });
  expect(codeTab).toBeInTheDocument();
  fireEvent.click(codeTab);
  const viewport = screen.getByTestId('code-viewport');
  expect(viewport).toHaveTextContent("print('running python snippet')");
  // Syntax highlighting marks the builtins distinctly inside the block.
  expect(viewport.querySelector('.hljs-built_in')).toHaveTextContent('print');
});

it('renders structured result separately from logs with outcome status', () => {
  render(
    <CommandRunCard
      toolName="run_code"
      language="python"
      status="completed"
      exitCode={0}
      stdout="log line"
      resultStatus="ok"
      envelopeStatus="success"
      structuredResult={'{\n  "answer": 42\n}'}
    />
  );
  fireEvent.click(screen.getByTestId('command-run-card').querySelector('summary')!);
  expect(screen.getByTestId('result-status')).toHaveTextContent('Structured result: ok');
  expect(screen.getByTestId('result-status')).toHaveTextContent('outcome success');
  const resultViewport = screen.getByTestId('result-viewport');
  // The friendly view renders the payload fields, not raw envelope JSON.
  expect(resultViewport).toHaveTextContent('answer');
  expect(resultViewport).toHaveTextContent('42');
  expect(resultViewport.textContent).not.toContain('schemaVersion');
  // Raw JSON stays one disclosure away.
  expect(resultViewport.querySelector('.raw-result-details summary')).toHaveTextContent('Raw result JSON');
  fireEvent.click(screen.getByRole('button', { name: 'Output' }));
  expect(screen.getByText('log line')).toBeInTheDocument();
});

it('truncates multiline command to first line in header title', () => {
  const multilineCmd = "git status\ngit diff\nnpm test";
  render(
    <CommandRunCard
      toolName="execute_command"
      command={multilineCmd}
      status="completed"
    />
  );
  expect(screen.getByText('git status')).toBeInTheDocument();
  expect(screen.queryByText('git status\ngit diff\nnpm test')).not.toBeInTheDocument();
});


it('reports failed structured validation even when the process exited zero', () => {
  render(<CommandRunCard toolName="execute_command" command="produce report" status="failed" isError exitCode={0} error="Expected result missing" />);
  expect(screen.getByTestId('status-failed')).toHaveTextContent('Failed');
  expect(screen.queryByText('finished')).not.toBeInTheDocument();
  expect(screen.getByText('Exit 0')).toBeInTheDocument();
});

it('renders stderr as a separate labelled channel without implying failure', () => {
  render(
    <CommandRunCard
      toolName="execute_command"
      command="npm install"
      status="completed"
      exitCode={0}
      stdout="added 3 packages"
      stderr="npm warn deprecated pkg@1.0.0"
    />
  );
  expect(screen.getByTestId('status-exit-0')).toBeInTheDocument();
  fireEvent.click(screen.getByTestId('command-run-card').querySelector('summary')!);
  const viewport = screen.getByTestId('terminal-viewport');
  const channels = viewport.querySelectorAll('.output-channel');
  expect(channels).toHaveLength(2);
  expect(channels[0]).toHaveClass('channel-stdout');
  expect(channels[1]).toHaveClass('channel-stderr');
  expect(channels[1]).toHaveTextContent('npm warn deprecated pkg@1.0.0');
  // stderr alone did not change the successful exit status.
  expect(screen.queryByTestId('status-failed')).toBeNull();
});

it('shows only a stderr channel when the command produced no stdout', () => {
  render(
    <CommandRunCard
      toolName="execute_command"
      command="fail-check"
      status="failed"
      exitCode={2}
      stderr="error: missing file"
    />
  );
  const channels = screen.getByTestId('terminal-viewport').querySelectorAll('.output-channel');
  expect(channels).toHaveLength(1);
  expect(channels[0]).toHaveClass('channel-stderr');
  expect(channels[0]).toHaveTextContent('error: missing file');
});

it('strips unsupported escape sequences instead of rendering them literally', () => {
  render(
    <CommandRunCard
      toolName="execute_command"
      command="cursor-app"
      status="running"
      stdout={'\x1b[?25lhidden cursor\x1b[2Jdone'}
    />
  );
  const viewport = screen.getByTestId('terminal-viewport');
  expect(viewport).toHaveTextContent('hidden cursordone');
  expect(viewport.textContent).not.toContain('?25l');
  expect(viewport.textContent).not.toContain('[2J');
});

it('keeps color state across lines and resolves progress rewrites', () => {
  render(
    <CommandRunCard
      toolName="execute_command"
      command="build"
      status="running"
      stdout={'Progress: 10%\rProgress: 80%\n\x1b[31merror summary'}
    />
  );
  const viewport = screen.getByTestId('terminal-viewport');
  expect(viewport).toHaveTextContent('Progress: 80%');
  expect(viewport.textContent).not.toContain('10%');
  const err = screen.getByText('error summary');
  expect(err).toHaveStyle({ color: 'var(--terminal-red, #ef4444)' });
});

it('reveals a jump-to-latest button after the user scrolls away', () => {
  const longOutput = Array.from({ length: 60 }, (_, i) => `line ${i}`).join('\n');
  render(
    <CommandRunCard
      toolName="execute_command"
      command="tail-log"
      status="running"
      stdout={longOutput}
    />
  );
  const viewport = screen.getByTestId('terminal-viewport');
  expect(screen.queryByRole('button', { name: 'Jump to latest output' })).toBeNull();
  // Simulate the user scrolling up: jsdom defaults scrollHeight 0, so mock it.
  Object.defineProperty(viewport, 'scrollHeight', { value: 2000, configurable: true });
  Object.defineProperty(viewport, 'clientHeight', { value: 200, configurable: true });
  viewport.scrollTop = 0;
  fireEvent.scroll(viewport);
  const jump = screen.getByRole('button', { name: 'Jump to latest output' });
  fireEvent.click(jump);
  expect(viewport.scrollTop).toBe(2000);
  expect(screen.queryByRole('button', { name: 'Jump to latest output' })).toBeNull();
});

it('labels interrupted runs with a neutral badge', () => {
  render(
    <CommandRunCard
      toolName="execute_command"
      command="long-build"
      status="interrupted"
      stdout="partial output"
    />
  );
  expect(screen.getByTestId('status-interrupted')).toBeInTheDocument();
  expect(screen.getByText('Stopped · outcome unknown')).toBeInTheDocument();
});

it('bounds a large log to 500 DOM lines while keeping every page and full copy available', () => {
  const output = Array.from({ length: 1600 }, (_, i) => `record ${i + 1}`).join('\n');
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.assign(navigator, { clipboard: { writeText } });
  const { container } = render(<CommandRunCard toolName="execute_command" status="running" stdout={output} />);
  fireEvent.click(screen.getByRole('button', { name: 'Browse all 1600 lines' }));
  expect(container.querySelectorAll('.terminal-line').length).toBeLessThanOrEqual(500);
  expect(screen.getByText('record 1600')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Previous lines' }));
  expect(screen.getByText('record 1001')).toBeInTheDocument();
  expect(container.querySelectorAll('.terminal-line')).toHaveLength(500);
  fireEvent.click(screen.getByTitle('Copy terminal output'));
  expect(writeText).toHaveBeenCalledWith(output);
});
it('reveals failures after a running row was collapsed and keeps stop actionable', () => {
  const stop = vi.fn();
  const view = render(<CommandRunCard toolName="execute_command" command="test" status="running" onStop={stop} />);
  fireEvent.click(screen.getByRole('button', { name: 'Stop command' })); expect(stop).toHaveBeenCalledOnce();
  const row = screen.getByTestId('command-run-card');
  (row as HTMLDetailsElement).open = false;
  fireEvent(row, new Event('toggle'));
  view.rerender(<CommandRunCard toolName="execute_command" command="test" status="failed" exitCode={2} error="Test failed" />);
  expect(row).toHaveAttribute('open'); expect(screen.getByRole('alert')).toHaveTextContent('Test failed');
});
it('does not infer a search count from arbitrary log lines', () => {
  render(<CommandRunCard toolName="search_files" status="completed" stdout="Scanning…\nDone" />);
  expect(screen.queryByText('2 results')).toBeNull();
});

it('shows the live tail and freezes the visible lines while reading older output', () => {
  const output = Array.from({ length: 60 }, (_, i) => `entry ${i + 1}`).join('\n');
  const view = render(<CommandRunCard toolName="execute_command" status="running" stdout={output} />);
  expect(screen.getByText('entry 60')).toBeInTheDocument();
  expect(screen.queryByText('entry 1')).toBeNull();
  const viewport = screen.getByTestId('terminal-viewport');
  Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 2000 });
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 200 });
  viewport.scrollTop = 100; fireEvent.scroll(viewport);
  view.rerender(<CommandRunCard toolName="execute_command" status="running" stdout={`${output}\nentry 61`} />);
  expect(screen.queryByText('entry 61')).toBeNull();
  expect(viewport.scrollTop).toBe(100);
  // Completion must not strand the frozen window when Latest resumes following.
  view.rerender(<CommandRunCard toolName="execute_command" status="completed" stdout={`${output}\nentry 61`} />);
  fireEvent.click(screen.getByRole('button', { name: 'Jump to latest output' }));
  expect(screen.getByText('entry 61')).toBeInTheDocument();
});

it('defers completed command contents until inspection and preserves them on collapse', () => {
  const view = render(<CommandRunCard toolName="execute_command" command="cargo test" status="completed" stdout={'record\n'.repeat(3000)} />);
  expect(screen.queryByTestId('terminal-viewport')).toBeNull();
  const summary = screen.getByTestId('command-run-card').querySelector('summary')!;
  fireEvent.click(summary);
  const viewport = screen.getByTestId('terminal-viewport');
  fireEvent.click(summary);
  view.rerender(<CommandRunCard toolName="execute_command" command="cargo test" status="completed" stdout={'record\n'.repeat(3000)} />);
  expect(screen.getByTestId('terminal-viewport')).toBe(viewport);
});
