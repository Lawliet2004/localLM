import { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, Download, PanelLeftOpen, Pencil, Trash2, X } from 'lucide-react';
import { confirm } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from './lib/api';
import { defaultRuntimeConfig, type Bootstrap, type Message } from './lib/types';
import { Sidebar, type Page } from './components/Sidebar';
import { Chat } from './components/Chat';
import { Models } from './components/Models';
import { Catalog } from './components/Catalog';
import { Connectors } from './components/Connectors';
import './styles.css';

const initialData: Bootstrap = {
  conversations: [], config: defaultRuntimeConfig,
  preferences: { runtimePath: '', modelPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: 'You are a helpful local assistant. Be clear and accurate. If you do not know something, say so.' },
  runtime: { phase: 'stopped', message: 'No model loaded', modelPath: null },
};

export default function App() {
  const [data, setData] = useState(initialData);
  const [page, setPage] = useState<Page>('chat');
  const [activeId, setActiveId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [loading, setLoading] = useState(nativeAvailable);
  const [generating, setGenerating] = useState(false);
  const [modelBusy, setModelBusy] = useState(false);
  const [error, setError] = useState('');
  const [search, setSearch] = useState('');
  const [collapsed, setCollapsed] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [title, setTitle] = useState('');
  const [theme, setTheme] = useState(() => localStorage.getItem('locallm-theme') || 'dark');
  const selection = useRef(0);
  const active = data.conversations.find(item => item.id === activeId);
  const busy = generating || modelBusy || loading;

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
        event.preventDefault(); if (!busy) { selection.current++; setActiveId(null); setMessages([]); setPage('chat'); }
      }
    }
    window.addEventListener('keydown', keydown); return () => window.removeEventListener('keydown', keydown);
  }, [busy]);
  async function selectConversation(id: string) {
    const version = ++selection.current;
    setPage('chat'); setActiveId(id); setLoading(true); setError(''); setRenaming(false);
    try { const next = await api.messages(id); if (version === selection.current) setMessages(next); }
    catch (e) { setError(errorMessage(e)); }
    finally { if (version === selection.current) setLoading(false); }
  }
  async function send(content: string) {
    setGenerating(true); setError('');
    let id = activeId;
    try {
      if (!id) {
        const conversation = await api.createConversation(); id = conversation.id;
        setActiveId(id); setData(current => ({ ...current, conversations: [conversation, ...current.conversations] }));
      }
      const conversationId = id;
      setMessages(current => [...current, { id: 'pending-user', conversationId, role: 'user', content, reasoning: '', status: 'complete', createdAt: Date.now() }]);
      await api.sendMessage(conversationId, content, event => setMessages(current => {
        const existing = current.find(message => message.id === event.messageId);
        if (existing) return current.map(message => message.id === event.messageId ? { ...message, content: message.content + event.content, reasoning: message.reasoning + event.reasoning } : message);
        return [...current, { id: event.messageId, conversationId, role: 'assistant', content: event.content, reasoning: event.reasoning, status: 'streaming', createdAt: Date.now() }];
      }));
    } catch (e) { setError(errorMessage(e)); throw e; }
    finally {
      try { if (id) setMessages(await api.messages(id)); setData(await api.bootstrap()); }
      catch (e) { setError(errorMessage(e)); }
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
    try { await api.deleteConversation(activeId); setData(current => ({ ...current, conversations: current.conversations.filter(item => item.id !== activeId) })); setActiveId(null); setMessages([]); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function rename() {
    if (!activeId) return;
    try { await api.renameConversation(activeId, title); setData(current => ({ ...current, conversations: current.conversations.map(item => item.id === activeId ? { ...item, title: title.trim() } : item) })); setRenaming(false); }
    catch (e) { setError(errorMessage(e)); }
  }
  function exportChat() {
    const text = `# ${active?.title || 'Conversation'}\n\n` + messages.map(message => `## ${message.role}\n\n${message.content}\n`).join('\n');
    const url = URL.createObjectURL(new Blob([text], { type: 'text/markdown' }));
    const link = document.createElement('a'); link.href = url; link.download = 'conversation.md'; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  return <div className={`app ${collapsed ? 'sidebar-collapsed' : ''}`}>
    {!collapsed && <Sidebar page={page} onPage={setPage} conversations={data.conversations} activeId={activeId} onSelect={id => void selectConversation(id)} onNew={() => { selection.current++; setActiveId(null); setMessages([]); setPage('chat'); setError(''); }} busy={busy} search={search} onSearch={setSearch} theme={theme} onTheme={() => setTheme(theme === 'dark' ? 'light' : 'dark')} onCollapse={() => setCollapsed(true)} />}
    <main className="workspace"><header className="workspace-header"><div>{collapsed && <button className="icon-button" aria-label="Expand sidebar" onClick={() => setCollapsed(false)}><PanelLeftOpen size={18} /></button>}{renaming && page === 'chat' ? <form className="rename-form" onSubmit={event => { event.preventDefault(); void rename(); }}><input autoFocus aria-label="Conversation title" maxLength={160} value={title} onChange={e => setTitle(e.target.value)} /><button className="icon-button" aria-label="Save title"><Check size={16} /></button><button type="button" className="icon-button" aria-label="Cancel rename" onClick={() => setRenaming(false)}><X size={16} /></button></form> : <span className="workspace-title">{page === 'chat' ? active?.title || 'New conversation' : page === 'models' ? 'Models & runtime' : page === 'connectors' ? 'Connectors' : 'Skills'}</span>}</div><div className="header-actions">{page === 'chat' && active && <><button className="icon-button" title="Rename conversation" aria-label="Rename conversation" disabled={busy} onClick={() => { setTitle(active.title); setRenaming(true); }}><Pencil size={15} /></button><button className="icon-button" title="Export conversation" aria-label="Export conversation" disabled={!messages.length} onClick={exportChat}><Download size={15} /></button><button className="icon-button" title="Delete conversation" aria-label="Delete conversation" disabled={busy} onClick={() => void removeConversation()}><Trash2 size={15} /></button></>}<button className="model-selector" onClick={() => setPage('models')}><span className={`status-dot ${data.runtime.phase === 'ready' ? 'ready' : ''}`} />{data.runtime.phase === 'ready' ? 'Model loaded' : 'Select model'}<ChevronDown size={13} /></button></div></header>
      {!nativeAvailable && <div className="preview-banner">Browser preview · Open the desktop application to load models and save conversations.</div>}
      {error && <div className="error-banner" role="alert"><span>{error}</span><button className="icon-button" aria-label="Dismiss error" onClick={() => setError('')}><X size={16} /></button></div>}
      {page === 'chat' ? <Chat messages={messages} generating={generating} ready={nativeAvailable && data.runtime.phase === 'ready'} loading={loading} onSend={send} onCancel={() => { void api.cancelGeneration().catch(e => setError(errorMessage(e))); }} onConfigure={() => setPage('models')} /> : page === 'models' ? <Models config={data.config} preferences={data.preferences} runtime={data.runtime} busy={!nativeAvailable || busy} onLoad={() => void modelAction(true)} onUnload={() => void modelAction(false)} onSaveConfig={async config => { try { await api.saveConfig(config); setData(current => ({ ...current, config })); } catch (e) { setError(errorMessage(e)); throw e; } }} onSavePreferences={async preferences => { try { await api.savePreferences(preferences); setData(current => ({ ...current, preferences })); } catch (e) { setError(errorMessage(e)); throw e; } }} /> : page === 'connectors' ? <Connectors /> : <Catalog kind="skills" />}
    </main>
  </div>;
}

