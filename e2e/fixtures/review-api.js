// Appended to audit-api.js by review-acceptance.spec.ts. Only the native
// transport is replaced; all product components and interactions are real.
const reviewTime = Date.now() - 10000;
const reviewId = 'review-conversation';
const reviewHash = 'a'.repeat(64);
window.__review = { previewCalls: [], savedCopies: [], searchAttempts: 0, escapedArtifact: false };
conversations = [{ id: reviewId, title: 'Review acceptance conversation', updatedAt: reviewTime, providerId: null, modelId: null }];
tasks[reviewId] = { projectId: 'a', archived: false, pinned: false };
const reviewFile = index => `exports/result-${String(index).padStart(2, '0')}.${index === 16 ? 'html' : index === 0 ? 'csv' : 'txt'}`;
const reviewTool = (id, name, result, createdAt) => ({ id, conversationId: reviewId, role: 'tool', reasoning: '', status: 'complete', createdAt,
  content: JSON.stringify({ request: { connector: 'Harness', name, arguments: name === 'web_search' ? { question: 'fixture evidence' } : { language: 'python', code: '# fixture only' }, decision: 'allowed' }, result }) });
messages = [
  { id: 'review-user', conversationId: reviewId, role: 'user', content: 'Compare the fixture results and prepare files.', reasoning: '', status: 'complete', createdAt: reviewTime },
  reviewTool('review-web', 'web_search', { sources: [
    { id: 'A', url: 'https://evidence.example.test/report#alpha', title: 'Fixture evidence report', snippet: 'Alpha completed in 12 ms.', publishedAt: '2026-10-17' },
    { id: 'B', url: 'https://evidence.example.test/report#beta', title: 'Fixture evidence report', snippet: 'Beta completed in 18 ms.' },
    { id: 'bad', url: 'javascript:window.__review.escapedArtifact=true', title: 'Invalid source' },
  ] }, reviewTime + 100),
  reviewTool('review-files', 'run_code', { workspace: '/fixture/a', stdout: 'Prepared fixture files.', exitCode: 0,
    artifacts: Array.from({ length: 17 }, (_, index) => ({ path: reviewFile(index), sha256: reviewHash, bytes: 96 })) }, reviewTime + 200),
  { id: 'review-answer', conversationId: reviewId, role: 'assistant', content: '# Answer ready\n\nAlpha is **6 ms faster**. [A][B]\n\n| Sample | Latency |\n| --- | --- |\n| Alpha | 12 ms |\n| Beta | 18 ms |\n\n```python\ndef add(a, b):\n    return a + b\n```\n\n![Fixture chart](https://assets.example.test/chart.svg)', reasoning: 'Historical reasoning with no measured duration.', status: 'complete', createdAt: reviewTime + 300 },
];
if (window.__reviewFailedExecution) {
  const execution = messages.find(message => message.id === 'review-files');
  const value = JSON.parse(execution.content);
  value.result = { exitCode: 0, isError: true, status: 'failure', error: { code: 'missing-result', message: 'The process exited successfully but did not return its required structured result.' } };
  execution.content = JSON.stringify(value);
}
api.localReasoningCapability = async () => 'enable_thinking';
const reviewBootstrap = api.bootstrap;
api.bootstrap = async () => {
  const value = await reviewBootstrap();
  return { ...value, preferences: { ...value.preferences, runtimePath: '/fixture/llama-server', modelPath: '/fixture/model.gguf', reasoningMode: 'auto' } };
};
api.readRuntimeLog = async () => ({ content: 'Review fixture runtime ready. This is a browser transport fixture, not a native inference measurement.', truncated: false });
api.documentIndexStatus = async () => ({ enabled: false, paths: [], updatedAt: 0 });
api.saveArtifactCopy = async (path, content, mimeType) => { window.__review.savedCopies.push({ path, content, mimeType }); };
api.previewExecutionArtifact = async (root, path, sha256) => {
  window.__review.previewCalls.push({ root, path, sha256 });
  if (root !== '/fixture/a' || sha256 !== reviewHash) throw Error('Unbound artifact request');
  return path.endsWith('.csv') ? { mimeType: 'text/csv', content: 'Name,Latency\nAlpha,12\nBeta,18\n' }
    : { mimeType: 'text/html', content: '<h1>Literal artifact</h1><script>parent.__review.escapedArtifact=true</script>' };
};
api.searchSessions = async query => {
  window.__review.searchAttempts++;
  if (window.__review.searchAttempts === 1) throw Error('Fixture search service failed');
  return [{ conversationId: reviewId, conversationTitle: 'Recovered result', messageId: 'review-answer', role: 'assistant', excerpt: `Recovered excerpt for ${query}`, createdAt: reviewTime }];
};
