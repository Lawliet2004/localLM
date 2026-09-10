import { render, screen, fireEvent, act, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Chat } from './Chat';
import type { Message, ProviderConnection } from '../lib/types';

const apiMocks = vi.hoisted(() => ({
  getArtifact: vi.fn(),
}));
vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: {
    getArtifact: apiMocks.getArtifact,
  },
}));

const props = { generating: false, ready: true, loading: false, onSend: vi.fn(), onCancel: vi.fn(), onConfigure: vi.fn() };
describe('chat action reporting and submission', () => {
  it('attaches file contents, removes chips, and keeps tools inside the composer', async () => {
    const onSend = vi.fn(async (_content: string) => {});
    render(<Chat {...props} messages={[]} onSend={onSend} composerTools={<button type="button">Choose tools</button>} />);
    expect(screen.getByRole('button', { name: 'Choose tools' }).closest('form')).toHaveClass('composer');
    fireEvent.change(screen.getByLabelText('Attach files'), { target: { files: [new File(['hello from disk'], 'note.txt')] } });
    await screen.findByRole('button', { name: 'Remove note.txt' });
    fireEvent.click(screen.getByRole('button', { name: 'Send message' }));
    await waitFor(() => expect(onSend).toHaveBeenCalledTimes(1));
    expect(onSend.mock.calls[0][0]).toContain('hello from disk');
    expect(screen.queryByRole('button', { name: 'Remove note.txt' })).not.toBeInTheDocument();
  });
  it('retains files after a rejected send and isolates them between conversations', async () => {
    const onSend = vi.fn(async () => { throw new Error('Not accepted'); });
    const { rerender } = render(<Chat {...props} messages={[]} onSend={onSend} conversationKey="a" />);
    fireEvent.change(screen.getByLabelText('Attach files'), { target: { files: [new File(['hello'], 'note.txt')] } });
    await screen.findByRole('button', { name: 'Remove note.txt' });
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Send message' })); });
    expect(screen.getByRole('button', { name: 'Remove note.txt' })).toBeInTheDocument();
    rerender(<Chat {...props} messages={[]} conversationKey="b" />);
    expect(screen.queryByRole('button', { name: 'Remove note.txt' })).not.toBeInTheDocument();
    rerender(<Chat {...props} messages={[]} conversationKey="a" />);
    fireEvent.click(screen.getByRole('button', { name: 'Remove note.txt' }));
    expect(screen.getByRole('button', { name: 'Send message' })).toBeDisabled();
  });
  it('renders the recorded local server name with its stable identity', () => {
    const message: Message = { id:'tool', conversationId:'chat', role:'tool', reasoning:'', createdAt:0, status:'complete', content:JSON.stringify({request:{connector:'local-stable-id',localServerName:'Research server',name:'lookup',arguments:{query:'example'},authorization:'user approval decision'},result:{isError:false}}) };
    const { rerender } = render(<Chat {...props} messages={[message]} />);
    expect(screen.getByText('Research server · lookup')).toBeInTheDocument();
    fireEvent.click(screen.getByText('Research server · lookup'));
    expect(screen.getByText('local-stable-id')).toBeVisible();
    expect(screen.getByText('Authorization: user approval decision')).toBeVisible();
    // Older audit rows retain their existing label without needing live settings.
    rerender(<Chat {...props} messages={[{...message,content:JSON.stringify({request:{connector:'Legacy',name:'lookup'},result:{}})}]} />);
    expect(screen.getByText('Legacy · lookup')).toBeInTheDocument();
  });
  it('shows a reviewable unified diff for workspace edits', () => {
    const message: Message = { id:'edit', conversationId:'chat', role:'tool', reasoning:'', createdAt:0, status:'complete', content:JSON.stringify({request:{connector:'Workspace',name:'edit_file',arguments:{path:'note.txt'},decision:'allowed'},result:{replacements:1,diff:'--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n-old\n+new\n'}}) };
    render(<Chat {...props} messages={[message]} />);
    fireEvent.click(screen.getByText('Workspace · edit_file'));
    expect(screen.getByText('Diff')).toBeVisible();
    expect(screen.getByText(/--- a\/note\.txt/, { exact: false })).toBeVisible();
  });
  it('retries the failed prompt as a new send without changing the draft', async () => {
    const onSend = vi.fn(async (_content: string) => {});
    const messages: Message[] = [
      { id: 'u', conversationId: 'c', role: 'user', content: 'Original prompt', reasoning: '', status: 'complete', createdAt: 1 },
      { id: 'a', conversationId: 'c', role: 'assistant', content: 'Partial answer', reasoning: '', status: 'error', createdAt: 2 },
    ];
    render(<Chat {...props} messages={messages} onSend={onSend} />);
    fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'Unsent draft' } });
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Retry last prompt' })); });
    expect(onSend).toHaveBeenCalledWith('Original prompt');
    expect(screen.getByLabelText('Message')).toHaveValue('Unsent draft');
    expect(screen.getByText('Partial answer')).toBeInTheDocument();
  });
  it('labels measured context separately from draft changes', () => {
    render(<Chat {...props} messages={[]} contextUsage={{ inputTokens: 123, responseReserve: 512, contextLength: 8192 }} />);
    const usage = screen.getByLabelText('Last request context');
    expect(usage).toHaveTextContent('123 input + 512 response reserve');
    fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'A new draft that has not been counted' } });
    expect(usage).toHaveTextContent('123 input');
    expect(usage).toHaveTextContent('Draft changes are not included');
  });
  it.each([
    ['complete', { isError: true, message: 'Process failed' }, 'allowed', 'Failed'],
    ['complete', { isError: true }, 'denied', 'Denied'],
    ['interrupted', 'Cancelled', 'allowed', 'Stopped · outcome unknown'],
    ['complete', { isError: false }, 'allowed', 'Finished'],
  ] as const)('reports %s tool outcomes accurately (%s)', (status, result, decision, label) => {
    const message: Message = { id: 'tool', conversationId: 'chat', role: 'tool', reasoning: '', createdAt: 0, status,
      content: JSON.stringify({ request: { connector: 'Local', name: 'run_code', decision }, result }) };
    render(<Chat {...props} messages={[message]} />);
    expect(screen.getByText(label)).toBeInTheDocument();
  });
  it('blocks duplicate submission and preserves a new draft after rejection', async () => {
    let reject!: (error: Error) => void;
    const onSend = vi.fn(() => new Promise<void>((_, fail) => { reject = fail; }));
    render(<Chat {...props} messages={[]} onSend={onSend} />);
    const input = screen.getByLabelText('Message');
    fireEvent.change(input, { target: { value: 'First request' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    fireEvent.change(input, { target: { value: 'Next draft' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(onSend).toHaveBeenCalledTimes(1);
    await act(async () => reject(new Error('Rejected')));
    expect(input).toHaveValue('Next draft');
  });
});

it('shows a saved generation error next to the partial response', () => {
  const message: Message = { id: 'failed', conversationId: 'chat', role: 'assistant', content: 'Partial result', reasoning: '', status: 'error', createdAt: 0, error: 'Response token limit reached. Increase the response limit.' };
  render(<Chat {...props} messages={[message]} />);
  expect(screen.getByText('Partial result')).toBeInTheDocument();
  expect(screen.getByText('Response token limit reached. Increase the response limit.')).toBeInTheDocument();
});

it('renders live activity timeline during generation', () => {
  const message: Message = { id: 'user-1', conversationId: 'chat', role: 'user', content: 'Hello', reasoning: '', status: 'complete', createdAt: 0 };
  render(
    <Chat
      {...props}
      messages={[message]}
      generating={true}
      liveActivity={{ state: 'ExecutingTools', activity: 'Querying system_time…' }}
    />
  );
  expect(screen.getByRole('status')).toHaveTextContent('Querying system_time…');
  expect(screen.getByText('ExecutingTools')).toBeInTheDocument();
});

it('inspects bounded tool output artifact on demand', async () => {
  apiMocks.getArtifact.mockResolvedValueOnce({
    id: 'art-xyz',
    conversationId: 'chat',
    runId: 'run-1',
    stepId: 'step-0',
    toolName: 'read_file',
    sha256: 'abc123',
    content: 'Complete unabridged file content from artifact store',
    createdAt: 0,
  });

  const toolMsg: Message = {
    id: 'tool-1',
    conversationId: 'chat',
    role: 'tool',
    reasoning: '',
    status: 'complete',
    createdAt: 0,
    content: JSON.stringify({
      request: { connector: 'Workspace', name: 'read_file', arguments: { path: 'data.txt' } },
      result: {
        _bounded: true,
        _artifactId: 'art-xyz',
        _originalBytes: 10240,
        content: 'Excerpt...',
      },
    }),
  };

  render(<Chat {...props} messages={[toolMsg]} />);
  expect(screen.getByText(/Full result captured in artifact/)).toBeInTheDocument();
  expect(screen.getByText('art-xyz')).toBeInTheDocument();
  expect(screen.getByText('(10.0 KB)')).toBeInTheDocument();

  const inspectBtn = screen.getByRole('button', { name: 'Inspect artifact' });
  fireEvent.click(inspectBtn);

  await waitFor(() => expect(apiMocks.getArtifact).toHaveBeenCalledWith('art-xyz'));
  expect(await screen.findByText('Complete unabridged file content from artifact store')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Loaded' })).toBeInTheDocument();
});

it('opens model popover and triggers onSelectModel for local and remote models', async () => {
  const onSelectModel = vi.fn(async () => {});
  const providers: ProviderConnection[] = [
    {
      id: 'prov-openai',
      name: 'OpenAI',
      baseUrl: 'https://api.openai.com/v1',
      apiFormat: 'openai-chat-completions',
      hasApiKey: true,
      verified: true,
      lastTestedAt: 0,
      models: [
        { id: 'gpt-4o-mini', contextLength: 128000, maxOutputTokens: 4096, toolSupport: 'supported' },
      ],
    },
  ];

  render(
    <Chat
      {...props}
      messages={[]}
      providers={providers}
      onSelectModel={onSelectModel}
      modelLabel="Local · llama.cpp"
    />
  );

  const modelTrigger = screen.getByRole('button', { name: /Local · llama\.cpp/ });
  fireEvent.click(modelTrigger);

  expect(screen.getByRole('dialog', { name: 'Select model' })).toBeInTheDocument();
  expect(screen.getByText('OpenAI')).toBeInTheDocument();
  expect(screen.getByText('gpt-4o-mini')).toBeInTheDocument();

  // Select remote model
  fireEvent.click(screen.getByRole('button', { name: /gpt-4o-mini/ }));
  expect(onSelectModel).toHaveBeenCalledWith({ providerId: 'prov-openai', modelId: 'gpt-4o-mini' });

  // Re-open and select local model
  fireEvent.click(modelTrigger);
  fireEvent.click(screen.getByRole('button', { name: /Local model/ }));
  expect(onSelectModel).toHaveBeenCalledWith({ providerId: null, modelId: '' });
});

it('displays scoped chat error and allows dismissing it', () => {
  const onDismissError = vi.fn();
  const message: Message = { id: 'user-1', conversationId: 'chat', role: 'user', content: 'Hello', reasoning: '', status: 'complete', createdAt: 0 };
  const { rerender } = render(
    <Chat
      {...props}
      messages={[message]}
      chatError="Inference failure: context blown"
      onDismissError={onDismissError}
    />
  );

  expect(screen.getByRole('alert')).toHaveTextContent('Inference failure: context blown');
  const dismissBtn = screen.getByRole('button', { name: 'Dismiss error' });
  fireEvent.click(dismissBtn);
  expect(onDismissError).toHaveBeenCalledTimes(1);

  // Without chatError, alert is not present
  rerender(<Chat {...props} messages={[message]} chatError="" onDismissError={onDismissError} />);
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
});

