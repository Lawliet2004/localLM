// Appended after audit-api.js: seeds a GenUI conversation and provider IPC.
conversations = [{ id: 'c1', title: 'GenUI demo', updatedAt: Date.now(), providerId: null, modelId: null }];
messages = [
  { id: 'u1', conversationId: 'c1', role: 'user', content: 'Show me the download dashboard', reasoning: '', status: 'complete', createdAt: 1 },
  { id: 'a1', conversationId: 'c1', role: 'assistant', reasoning: '', status: 'complete', createdAt: 2,
    content: '## Download dashboard\n\nHere is your dashboard:\n\n```openui\nroot = Response([s, b])\ns = Columns([Stat("Downloaded", "2.07 GB"), Stat("Errors", "0")])\nb = Button("Show details")\n```\n\n## Report\n\n```html\n<h1>Weekly report</h1><p>Generated artifact preview.</p>\n```\n\n```python\nprint("done")\n```' },
  { id: 'u2', conversationId: 'c1', role: 'user', content: 'Add error tracking too', reasoning: '', status: 'complete', createdAt: 3 },
  { id: 'a2', conversationId: 'c1', role: 'assistant', reasoning: '', status: 'complete', createdAt: 4,
    content: '## Download dashboard\n\nUpdated version:\n\n```openui\nroot = Response([s, t])\ns = Columns([Stat("Downloaded", "2.09 GB"), Stat("Errors", "1")])\nt = Callout("One file failed a checksum.", "warning")\n```' },
];
api.localReasoningCapability = async () => 'enable_thinking';
api.saveProvider = async draft => ({ id: 'prov-1', name: draft.name, apiFormat: draft.apiFormat, baseUrl: draft.baseUrl, verified: false, lastTestedAt: null, models: draft.models ?? [], hasApiKey: Boolean(draft.apiKey) });
api.testProvider = async () => ({ verified: true, modelListSupported: true, models: ['llama3.3'], message: 'Connection verified. Model listing returned 1 model.' });
api.listProviderModels = async () => ({ id: 'prov-1', name: 'Ollama', apiFormat: 'openai-chat-completions', baseUrl: 'http://localhost:11434/v1', verified: true, lastTestedAt: Date.now(), hasApiKey: false, models: [{ id: 'llama3.3', contextLength: 128000, maxOutputTokens: 8192, toolSupport: 'supported', supportsImages: false }] });
api.hardwareStatus = async () => null;
api.modelDownloadInfo = async () => null;
api.searchHuggingFace = async () => [];
api.readRuntimeLog = async () => '';
api.retestToolCalling = async () => ({ ok: true, message: 'Tool calling verified' });
api.useInstalledModel = async () => {};
