import { render, screen, fireEvent, act, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Chat } from './Chat';
import type { Message, ProviderConnection } from '../lib/types';

const apiMocks = vi.hoisted(() => ({
  getArtifact: vi.fn(),
  contextPreflight: vi.fn(),
  compactConversation: vi.fn(),
  getTodos: vi.fn<() => Promise<any[]>>(async () => []),
}));
vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: {
    listInstalledModels: async () => [],
    compactionStatus: async () => ({ auto: true, keepLast: 10, checkpoint: null }),
    getArtifact: apiMocks.getArtifact,
    contextPreflight: apiMocks.contextPreflight,
    compactConversation: apiMocks.compactConversation,
    getTodos: apiMocks.getTodos,
  },
}));

afterEach(() => {
  apiMocks.getTodos.mockReset();
  apiMocks.getTodos.mockResolvedValue([]);
  apiMocks.contextPreflight.mockReset();
  apiMocks.contextPreflight.mockResolvedValue({ available: false });
  apiMocks.compactConversation.mockReset();
  apiMocks.compactConversation.mockResolvedValue({ note: '', checkpoint: null });
});

const props = { generating: false, ready: true, loading: false, onSend: vi.fn(), onCancel: vi.fn(), onConfigure: vi.fn() };
describe('chat action reporting and submission', () => {
  it('pause and resume call the generation controls while a response is running', () => {
    const onPause = vi.fn();
    const onResume = vi.fn();
    const { rerender } = render(<Chat {...props} messages={[]} generating onPause={onPause} onResume={onResume} />);
    fireEvent.click(screen.getByRole('button', { name: 'Pause response' }));
    expect(onPause).toHaveBeenCalledOnce();
    rerender(<Chat {...props} messages={[]} generating={false} paused onPause={onPause} onResume={onResume} />);
    fireEvent.click(screen.getByRole('button', { name: 'Resume response' }));
    expect(onResume).toHaveBeenCalledOnce();
  });
  it('shows a collapsible plan checklist and hides todo harness cards', async () => {
    apiMocks.getTodos.mockResolvedValue([
      { text: 'Search papers', status: 'completed', updatedAt: 1 },
      { text: 'Open the source', status: 'in_progress', updatedAt: 2 },
    ]);
    const user: Message = { id: 'u', conversationId: 'c1', role: 'user', status: 'complete', content: 'Research this', reasoning: '', createdAt: 1 };
    const todoTool: Message = {
      id: 't', conversationId: 'c1', role: 'tool', status: 'complete', reasoning: '', createdAt: 2,
      content: JSON.stringify({ request: { connector: 'Harness', name: 'todo_write', arguments: { todos: [] } }, result: { saved: 2 } }),
    };
    render(<Chat {...props} conversationKey="c1" messages={[user, todoTool]} generating planMode />);
    const checklist = await screen.findByLabelText('Plan checklist');
    expect(checklist).toHaveTextContent('1 of 2');
    expect(screen.getByText('Search papers')).toBeInTheDocument();
    expect(screen.queryByText('Harness · todo_write')).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: /Plan\s*1 of 2/i }));
    expect(screen.queryByRole('list')).not.toBeInTheDocument();
  });
  it('toggles plan mode from the composer', () => {
    const onPlanModeChange = vi.fn();
    const { rerender } = render(<Chat {...props} messages={[]} planMode={false} onPlanModeChange={onPlanModeChange} />);
    const toggle = screen.getByRole('button', { name: 'Plan mode' });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    expect(toggle).not.toHaveTextContent('Plan');
    fireEvent.click(toggle);
    expect(onPlanModeChange).toHaveBeenCalledWith(true);
    rerender(<Chat {...props} messages={[]} planMode onPlanModeChange={onPlanModeChange} />);
    const active = screen.getByRole('button', { name: 'Plan mode' });
    expect(active).toHaveAttribute('aria-pressed', 'true');
    expect(active).toHaveTextContent('Plan');
    expect(screen.getByLabelText('Message')).toHaveAttribute('placeholder', 'Describe the task to plan…');
  });
  it('shows unavailable tools as a notice without blocking ordinary chat', () => {
    render(<Chat {...props} messages={[]} draft="Hello" chatNotice="Unavailable for this reply: parallel-web, deepwiki." />);
    expect(screen.getByRole('status')).toHaveTextContent('parallel-web, deepwiki');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Send message' })).toBeEnabled();
  });
  it('shows edit cards when the persisted assistant row precedes its tool results', () => {
    const user: Message = {id:'u',conversationId:'chat',role:'user',status:'complete',content:'Edit styles',reasoning:'',createdAt:1};
    const assistant: Message = {...user,id:'a',role:'assistant',content:'Updated the styles.'};
    const tool: Message = {...user,id:'t',role:'tool',content:JSON.stringify({request:{connector:'Workspace',name:'edit_file',arguments:{path:'styles.css'}},result:{diff:'--- a/styles.css\n+++ b/styles.css\n@@ -1 +1 @@\n-old\n+new'}})};
    render(<Chat {...props} messages={[user,assistant,tool]} />);
    expect(screen.getByText('Edited styles.css')).toBeVisible();
    expect(screen.getByText('Review')).toBeVisible();
  });
  it('shows send failures before the first message is saved', () => {
    render(<Chat {...props} messages={[]} chatError="Connector parallel-web is not connected." />);
    expect(screen.getByRole('alert')).toHaveTextContent('Connector parallel-web is not connected.');
  });
  it('shows preparation activity before the first message is saved', () => {
    render(<Chat {...props} messages={[]} generating liveActivity={{ activity: 'Preparing request…' }} />);
    expect(screen.getByRole('status')).toHaveTextContent('Preparing request…');
  });
  it('pastes images from clipboard via Ctrl+V', async () => {
    const onSend = vi.fn(async (_content: string) => {});
    render(<Chat {...props} messages={[]} onSend={onSend} />);
    const input = screen.getByLabelText('Message');
    const pngHeader = new Uint8ClampedArray([137, 80, 78, 71, 13, 10, 26, 10]);
    const blob = new Blob([pngHeader], { type: 'image/png' });
    const file = new File([blob], 'pasted.png', { type: 'image/png' });
    Object.defineProperty(input, 'files', {
      value: [file],
      writable: false,
    });
    fireEvent.paste(input, { clipboardData: { files: [file] } });
    await screen.findByRole('button', { name: /Remove pasted\.png/ });
    expect(onSend).not.toHaveBeenCalled();
  });
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
  it('renders web search as a searched-the-web card with source chips', () => {
    const message: Message = {
      id: 'search', conversationId: 'chat', role: 'tool', reasoning: '', createdAt: 0, status: 'complete',
      content: JSON.stringify({
        request: { connector: 'Harness', name: 'web_search', arguments: { question: 'fusion yield' }, decision: 'allowed' },
        result: { answer: 'Yield exceeded one.', sources: { S1: { title: 'NIF', url: 'https://example.com/nif' } } },
      }),
    };
    render(<Chat {...props} messages={[message]} />);
    expect(screen.getByText('Searched the web')).toBeInTheDocument();
    expect(screen.getByText('fusion yield')).toBeInTheDocument();
    expect(screen.getByText('1 result')).toBeInTheDocument();
    fireEvent.click(screen.getByText('fusion yield'));
    expect(screen.getByRole('link', { name: 'NIF' })).toHaveAttribute('href', 'https://example.com/nif');
  });
  it('shows a reviewable unified diff for workspace edits', () => {
    const message: Message = { id:'edit', conversationId:'chat', role:'tool', reasoning:'', createdAt:0, status:'complete', content:JSON.stringify({request:{connector:'Workspace',name:'edit_file',arguments:{path:'note.txt'},decision:'allowed'},result:{replacements:1,diff:'--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n-old\n+new\n'}}) };
    render(<Chat {...props} messages={[message]} />);
    fireEvent.click(screen.getByText('note.txt'));
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
it('shows measured context usage in the context popover', () => {
     render(<Chat {...props} messages={[]} contextUsage={{ inputTokens: 123, responseReserve: 512, contextLength: 8192 }} />);
     const popover = screen.getByRole('dialog', { name: 'Context usage and compaction' });
     expect(popover).toHaveTextContent('123 input + 512 reserved / 8,192 tokens');
   });
    it('warns when the draft overflows the context window', async () => {
      apiMocks.contextPreflight.mockResolvedValue({ available: true, breakdown: { total: 9000, instructions: 50, tools: 30, history: 4000, draft: 4920, responseReserve: 512, contextLength: 8192, exact: true, fits: false, overflow: 'Over the context limit — compact history or shorten the draft.' } });
      render(<Chat {...props} messages={[]} ready={true} draft="test prompt" />);
      await waitFor(() => {
        expect(screen.getByRole('alert')).toHaveTextContent('Over the context limit');
      });
    });
    it('triggers manual compaction from the context popover', async () => {
      apiMocks.compactConversation.mockResolvedValue({ note: 'Compacted 4 messages into artifact art-1', checkpoint: { cutoff: 12345 } });
      const messages: Message[] = [
        { id: '1', conversationId: 'c1', role: 'user', content: 'Turn 1', reasoning: '', status: 'complete', createdAt: 1 },
        { id: '2', conversationId: 'c1', role: 'assistant', content: 'Reply 1', reasoning: '', status: 'complete', createdAt: 2 },
        { id: '3', conversationId: 'c1', role: 'user', content: 'Turn 2', reasoning: '', status: 'complete', createdAt: 3 },
      ];
      render(<Chat {...props} conversationKey="c1" messages={messages} ready={true} draft="Next message" />);
      const compactBtn = await screen.findByRole('button', { name: 'Compact history' });
      fireEvent.click(compactBtn);
      await waitFor(() => expect(apiMocks.compactConversation).toHaveBeenCalledWith('c1'));
      expect(await screen.findByText('Compacted 4 messages into artifact art-1')).toBeInTheDocument();
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

it('renders reasoning block with thinking indicator while streaming and thought summary when complete', () => {
  const streamingMsg: Message = {
    id: 'asst-1',
    conversationId: 'chat',
    role: 'assistant',
    content: 'Final answer',
    reasoning: 'Analyzing query parameters...',
    status: 'streaming',
    createdAt: Date.now(),
  };

  const { rerender } = render(<Chat {...props} messages={[streamingMsg]} generating={true} />);

  expect(screen.getByText(/Thinking \(/)).toBeInTheDocument();
  expect(screen.getByText('Analyzing query parameters...')).toBeInTheDocument();

  const completedMsg: Message = {
    ...streamingMsg,
    status: 'complete',
  };

  rerender(<Chat {...props} messages={[completedMsg]} generating={false} />);
  expect(screen.getByText(/Thought/)).toBeInTheDocument();
});

it('renders copy button on markdown code blocks', async () => {
  const writeTextMock = vi.fn().mockResolvedValue(undefined);
  Object.assign(navigator, { clipboard: { writeText: writeTextMock } });

  const codeMsg: Message = {
    id: 'asst-2',
    conversationId: 'chat',
    role: 'assistant',
    content: "```typescript\nconst greeting = 'hello';\n```",
    reasoning: '',
    status: 'complete',
    createdAt: 0,
  };

  render(<Chat {...props} messages={[codeMsg]} />);

  const copyBtn = screen.getByRole('button', { name: 'Copy code block' });
  expect(copyBtn).toBeInTheDocument();
  fireEvent.click(copyBtn);

  expect(writeTextMock).toHaveBeenCalledWith("const greeting = 'hello';");
  expect(await screen.findByText('Copied!')).toBeInTheDocument();
});


