import { createRoot } from 'react-dom/client';
import { Chat } from '../src/components/Chat';
import { Trajectory } from '../src/components/Trajectory';
import { api } from '../src/lib/api';
import type { Message } from '../src/lib/types';

export function mountChatPreview() {
  api.getConversationRun = async () => ({ id: 'preview-run', conversationId: 'preview', status: 'completed', createdAt: 1000, updatedAt: 232000 });
  const messages: Message[] = [
    { id: 'user', conversationId: 'preview', role: 'user', content: 'Make the chat output feel like Codex.', reasoning: '', status: 'complete', createdAt: 1000 },
    { id: 'tool', conversationId: 'preview', role: 'tool', content: JSON.stringify({ request: { connector: 'Workspace', name: 'edit_file', arguments: { path: 'src/components/Chat.tsx' }, decision: 'allowed' }, result: { diff: '--- a/src/components/Chat.tsx\n+++ b/src/components/Chat.tsx\n@@ -1 +1,2 @@\n-old\n+new\n+another' } }), reasoning: '', status: 'complete', createdAt: 2000 },
    { id: 'answer', conversationId: 'preview', role: 'assistant', content: 'The transcript now keeps progress in a quiet disclosure and leaves the answer easy to read. File changes stay reviewable below the response.', reasoning: 'I inspected the transcript and grouped work by turn.', status: 'complete', createdAt: 232000 },
  ];
  const host = document.createElement('div');
  host.className = 'app sidebar-collapsed';
  host.style.cssText = 'position:fixed;inset:0;z-index:10000;background:var(--background)';
  host.innerHTML = '<main class="workspace" style="grid-row:1 / -1;min-height:0"></main>';
  document.body.append(host);
  createRoot(host.querySelector('main')!).render(<><Chat messages={messages} conversationKey="preview" generating={false} ready={false} loading={false} onSend={async () => {}} onCancel={() => {}} onConfigure={() => {}} /><Trajectory conversationId="preview" /></>);
}
