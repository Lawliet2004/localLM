import { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, Download, PanelLeftOpen, Pencil, Trash2, X } from 'lucide-react';
import { confirm, save } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from './lib/api';
import { defaultRuntimeConfig, type Bootstrap, type Message, type ToolApproval, type ToolSelection } from './lib/types';
import { Sidebar, type Page } from './components/Sidebar';
import { Chat } from './components/Chat';
import { Models } from './components/Models';
import { Skills } from './components/Skills';
import { Execution } from './components/Execution';
import { Connectors } from './components/Connectors';
import { ApprovalDialog, ToolPicker } from './components/ToolControls';
import './styles.css';

const initialData: Bootstrap = {
  conversations: [], config: defaultRuntimeConfig,
  preferences: { runtimePath: '', modelPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: 'You are a helpful local assistant. Be clear and accurate. If you do not know something, say so.' },
  runtime: { phase: 'stopped', message: 'No model loaded', modelPath: null },
};

export default function App() {
  const [selectedConnectors, setSelectedConnectors] = useState<string[]>([]);
  const [selectedTools, setSelectedTools] = useState<ToolSelection[]>([]);
  const [approval, setApproval] = useState<ToolApproval | null>(null);
  const [data, setData] = useState(initialData);
  const [page, setPage] = useState<Page>('chat');
  const [activeId, setActiveId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [loading, setLoading] = useState(nativeAvailable);
  const [generating, setGenerating] = useState(false);
  const [modelBusy, setModelBusy] = useState(false);
  const [savingTools, setSavingTools] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exportedPath, setExportedPath] = useState('');
  const toolSave = useRef(false);
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');
  const [collapsed, setCollapsed] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState('');
  const [theme, setTheme] = useState(() => localStorage.getItem('locallm-theme') || 'dark');
  const selection = useRef(0);
  const active = data.conversations.find(item => item.id === activeId);
  const busy = generating || modelBusy || loading || savingTools || exporting;
  function newConversation() {
    selection.current++; setActiveId(null); setMessages([]); setPage('chat'); setError(''); setRenaming(false);
    setSelectedConnectors([]); setSelectedTools([]);
  }
  async function changeTools(sources: string[], tools: ToolSelection[]) {
    if (busy || toolSave.current) return;
    toolSave.current = true; setSavingTools(true); setError('');
    try {
      if (activeId) await api.saveConversationTools(activeId, { sources, tools });
      setSelectedConnectors(sources); setSelectedTools(tools);
    } catch (e) { setError(errorMessage(e)); }
    finally { toolSave.current = false; setSavingTools(false); }
  }

  useEffect(() => { document.documentElement.dataset.theme = theme; localStorage.setItem('locallm-theme', theme); }, [theme]);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.bootstrap().then(value => { if (!disposed) setData(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, []);
  useEffect(() => {
    function keydown(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'n') {
        event.preventDefault(); if (!busy) newConversation();
      }
    }
    window.addEventListener('keydown', keydown); return () => window.removeEventListener('keydown', keydown);
  }, [busy]);
  async function selectConversation(id: string) {
    if (busy || toolSave.current) return;
    const version = ++selection.current;
    setPage('chat'); setActiveId(id); setLoading(true); setError(''); setRenaming(false);
    setMessages([]); setSelectedConnectors([]); setSelectedTools([]);
    try {
      const [next, tools] = await Promise.all([api.messages(id), api.conversationTools(id)]);
      if (version === selection.current) { setMessages(next); setSelectedConnectors(tools.sources); setSelectedTools(tools.tools); }
    }
    catch (e) { if (version === selection.current) { setError(errorMessage(e)); setActiveId(null); } }
    finally { if (version === selection.current) setLoading(false); }
  }
  async function send(content: string) {
    if (busy || toolSave.current) throw new Error('Wait for the active operation before sending a message.');
    setGenerating(true); setError('');
    let id = activeId;
    const previousIds = new Set(messages.map(message => message.id));
    try {
      if (!id) {
        const conversation = await api.createConversation(); id = conversation.id;
        setActiveId(id); setData(current => ({ ...current, conversations: [conversation, ...current.conversations] }));
      }
      await api.saveConversationTools(id, { sources: selectedConnectors, tools: selectedTools });
      const conversationId = id;
      setMessages(current => [...current, { id: 'pending-user', conversationId, role: 'user', content, reasoning: '', status: 'complete', createdAt: Date.now() }]);
      await api.sendMessage(conversationId, content, event => { if ('approval' in event) setApproval(event.approval || null); setMessages(current => {
        const existing = current.find(message => message.id === event.messageId);
        if (existing) return current.map(message => message.id === event.messageId ? { ...message, content: message.content + event.content, reasoning: message.reasoning + event.reasoning } : message);
        return [...current, { id: event.messageId, conversationId, role: 'assistant', content: event.content, reasoning: event.reasoning, status: 'streaming', createdAt: Date.now() }];
      }); }, selectedConnectors, selectedTools);
    } catch (e) {
      setError(errorMessage(e));
      // Restore the draft only when persistence confirms the message was not accepted.
      // An inference failure can happen after the user message has already been saved.
      if (!id) throw e;
      let saved: Message[];
      try { saved = await api.messages(id); }
      catch {
        setError(`${errorMessage(e)} Could not verify whether your message was saved. Reopen the conversation before sending it again.`);
        return;
      }
      setMessages(saved);
      if (!saved.some(message => message.role === 'user' && !previousIds.has(message.id) && message.content === content.trim())) throw e;
    }
    finally {
      try { if (id) setMessages(await api.messages(id)); setData(await api.bootstrap()); }
      catch (e) { setError(errorMessage(e)); }
      setApproval(null);
      setGenerating(false);
    }
  }
  async function modelAction(load: boolean) {
    setModelBusy(true); setError('');
    try { const runtime = await (load ? api.loadModel() : api.unloadModel()); setData(current => ({ ...current, runtime })); }
    catch (e) { setError(errorMessage(e)); setData(current => ({ ...current, runtime: { phase: 'error', message: errorMessage(e), modelPath: null } })); }
    finally { setModelBusy(false); }
  }
  async function removeConversation() {
    if (!activeId || busy) return;
    if (!await confirm('This permanently deletes this conversation from your device.', { title: 'Delete conversation?', kind: 'warning' })) return;
    try { await api.deleteConversation(activeId); setData(current => ({ ...current, conversations: current.conversations.filter(item => item.id !== activeId) })); newConversation(); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function rename() {
    if (!activeId) return;
    try { await api.renameConversation(activeId, title); setData(current => ({ ...current, conversations: current.conversations.map(item => item.id === activeId ? { ...item, title: title.trim() } : item) })); setRenaming(false); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function exportChat() {
    if (!activeId || busy) return;
    setExporting(true); setError(''); setExportedPath('');
    try {
      const path = await save({ title: 'Export conversation', defaultPath: `conversation-${activeId.slice(0, 8)}.md`, filters: [
        { name: 'Markdown', extensions: ['md'] }, { name: 'JSON (complete conversation data)', extensions: ['json'] },
      ] });
      if (path) { await api.exportConversation(activeId, path); setExportedPath(path); }
    } catch (e) { setError(errorMessage(e)); }
    finally { setExporting(false); }
  }
  return <div className={`app ${collapsed ? 'sidebar-collapsed' : ''}`}>
    {!collapsed && <Sidebar page={page} onPage={setPage} conversations={data.conversations} activeId={activeId} onSelect={id => void selectConversation(id)} onNew={newConversation} busy={busy} search={search} onSearch={setSearch} theme={theme} onTheme={() => setTheme(theme === 'dark' ? 'light' : 'dark')} onCollapse={() => setCollapsed(true)} />}
    {approval && <ApprovalDialog key={approval.id} request={approval} onResolve={allow => api.resolveToolApproval(approval.id, allow)} />}
    <main className="workspace"><header className="workspace-header"><div>{collapsed && <button className="icon-button" aria-label="Expand sidebar" onClick={() => setCollapsed(false)}><PanelLeftOpen size={18} /></button>}{renaming && page === 'chat' ? <form className="rename-form" onSubmit={event => { event.preventDefault(); void rename(); }}><input autoFocus aria-label="Conversation title" maxLength={160} value={title} onChange={e => setTitle(e.target.value)} /><button className="icon-button" aria-label="Save title"><Check size={16} /></button><button type="button" className="icon-button" aria-label="Cancel rename" onClick={() => setRenaming(false)}><X size={16} /></button></form> : <span className="workspace-title">{page === 'chat' ? active?.title || 'New conversation' : page === 'models' ? 'Models & runtime' : page === 'connectors' ? 'Connectors' : page === 'execution' ? 'Execution' : 'Skills'}</span>}</div><div className="header-actions">{page === 'chat' && active && <><button className="icon-button" title="Rename conversation" aria-label="Rename conversation" disabled={busy} onClick={() => { setTitle(active.title); setRenaming(true); }}><Pencil size={15} /></button><button className="icon-button" title="Export conversation" aria-label="Export conversation" disabled={!messages.length || busy} onClick={() => void exportChat()}><Download size={15} /></button><button className="icon-button" title="Delete conversation" aria-label="Delete conversation" disabled={busy} onClick={() => void removeConversation()}><Trash2 size={15} /></button></>}<button className="model-selector" onClick={() => setPage('models')}><span className={`status-dot ${data.runtime.phase === 'ready' ? 'ready' : ''}`} />{data.runtime.phase === 'ready' ? 'Model loaded' : 'Select model'}<ChevronDown size={13} /></button></div></header>
      {!nativeAvailable && <div className="preview-banner">Browser preview · Open the desktop application to load models and save conversations.</div>}
      {exportedPath && <div className="preview-banner" role="status">Saved conversation to {exportedPath}<button className="icon-button" aria-label="Dismiss export confirmation" onClick={() => setExportedPath('')}><X size={16} /></button></div>}
      {error && <div className="error-banner" role="alert"><span>{error}</span><button className="icon-button" aria-label="Dismiss error" onClick={() => setError('')}><X size={16} /></button></div>}
      {page === 'chat' && <ToolPicker selectedTools={selectedTools} onToolsChange={tools => void changeTools(selectedConnectors, tools)} selected={selectedConnectors} onChange={sources => void changeTools(sources, selectedTools)} busy={busy} />}
      {page === 'chat' ? <Chat messages={messages} generating={generating} ready={nativeAvailable && data.runtime.phase === 'ready'} loading={loading} disabled={savingTools || exporting} onSend={send} onCancel={() => { void api.cancelGeneration().catch(e => setError(errorMessage(e))); }} onConfigure={() => setPage('models')} /> : page === 'models' ? <Models config={data.config} preferences={data.preferences} runtime={data.runtime} busy={!nativeAvailable || busy} onLoad={() => void modelAction(true)} onUnload={() => void modelAction(false)} onSaveConfig={async config => { try { await api.saveConfig(config); setData(current => ({ ...current, config })); } catch (e) { setError(errorMessage(e)); throw e; } }} onSavePreferences={async preferences => { try { await api.savePreferences(preferences); setData(current => ({ ...current, preferences })); } catch (e) { setError(errorMessage(e)); throw e; } }} /> : page === 'execution' ? <Execution /> : page === 'connectors' ? <Connectors /> : <Skills />}
    </main>
  </div>;
}
