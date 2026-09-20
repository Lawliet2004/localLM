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
  expect(screen.getByText('2 results')).toBeInTheDocument();
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
  expect(screen.getByText('-old line')).toBeInTheDocument();
  expect(screen.getByText('+new line')).toBeInTheDocument();
});

it('supports folding long outputs exceeding 30 lines', () => {
  const longOutput = Array.from({ length: 45 }, (_, i) => `log line ${i + 1}`).join('\n');
  render(
    <CommandRunCard
      toolName="execute_command"
      command="dump-logs"
      status="running"
      stdout={longOutput}
    />
  );
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
  const codeTab = screen.getByRole('button', { name: 'Code' });
  expect(codeTab).toBeInTheDocument();
  fireEvent.click(codeTab);
  expect(screen.getByTestId('code-viewport')).toBeInTheDocument();
  expect(screen.getByText("print('running python snippet')")).toBeInTheDocument();
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
  expect(screen.getByTestId('result-status')).toHaveTextContent('Structured result: ok');
  expect(screen.getByTestId('result-status')).toHaveTextContent('outcome success');
  expect(screen.getByTestId('result-viewport')).toBeInTheDocument();
  expect(screen.getByText(/"answer": 42/)).toBeInTheDocument();
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

