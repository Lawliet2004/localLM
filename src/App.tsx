import { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, Download, PanelLeftOpen, Pencil, Trash2, X } from 'lucide-react';
import { confirm, save } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from './lib/api';
import { defaultRuntimeConfig, type Bootstrap, type Message, type ToolApproval, type ToolSelection, type AccessMode, type ContextUsage, type ModelSelection, type RememberedTools } from './lib/types';
import { Sidebar, type Page } from './components/Sidebar';
import type { Attachment } from './lib/attachments';
import { Chat } from './components/Chat';
import { Models } from './components/Models';
import { Skills } from './components/Skills';
import { Execution } from './components/Execution';
import { Connectors } from './components/Connectors';
import { ApprovalDialog, ToolPicker } from './components/ToolControls';
import { Trajectory } from './components/Trajectory';
import { TodosPanel } from './components/TodosPanel';
import { SessionsPanel } from './components/SessionsPanel';
import { PresetPicker } from './components/PresetPicker';
import { Memory } from './components/Memory';
import { Plugins } from './components/Plugins';
import './styles.css';
import { loadDraft, saveDraft, moveNewDraft } from './lib/drafts';

const initialData: Bootstrap = {
  conversations: [], config: defaultRuntimeConfig,
  preferences: { runtimePath: '', modelPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: 'You are a helpful local assistant. Be clear and accurate. If you do not know something, say so.' },
  runtime: { phase: 'stopped', message: 'No model loaded', modelPath: null },
  rememberedTools: { sources: [], tools: [] },
  providers: [], preferredModel: { providerId: null, modelId: '' },
};
function normalizeBootstrap(value: Bootstrap): Bootstrap {
  return { ...value, providers: value.providers ?? [], preferredModel: value.preferredModel ?? { providerId: null, modelId: '' }, rememberedTools: value.rememberedTools ?? { sources: [], tools: [] } };
}

export default function App() {
  const [attachmentDrafts, setAttachmentDrafts] = useState<Record<string, Attachment[]>>({});
  const draftId = useRef<string | null>(null);
  const draftValue = useRef(loadDraft(null));
  const [draft, setDraft] = useState(draftValue.current);
  function changeDraft(value: string | ((previous: string) => string)) {
    const next = typeof value === 'function' ? value(draftValue.current) : value;
    draftValue.current = next; setDraft(next);
    try { saveDraft(draftId.current, next); } catch (e) { setError(errorMessage(e)); }
  }
  function switchDraft(id: string | null) {
    draftId.current = id; draftValue.current = loadDraft(id); setDraft(draftValue.current);
  }
  const [selectedConnectors, setSelectedConnectors] = useState<string[]>([]);
  const [accessMode, setAccessMode] = useState<AccessMode>('ask');
  const [selectedTools, setSelectedTools] = useState<ToolSelection[]>([]);
  // Last explicitly chosen selection for future chats; restored at startup and
  // applied by newConversation. Existing conversations never read from it.
  const rememberedTools = useRef<RememberedTools>({ sources: [], tools: [] });
  const [approval, setApproval] = useState<ToolApproval | null>(null);
  const [data, setData] = useState(initialData);
  const [page, setPage] = useState<Page>('chat');
  const [activeId, setActiveId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [contextUsage, setContextUsage] = useState<{ conversationId: string; usage: ContextUsage } | null>(null);
  const [loading, setLoading] = useState(nativeAvailable);
  const [generating, setGenerating] = useState(false);
  const [liveActivity, setLiveActivity] = useState<{ state?: string; activity?: string } | null>(null);
  const [chatError, setChatError] = useState('');
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
  const modelSelection: ModelSelection = active ? { providerId: active.providerId, modelId: active.modelId ?? '' } : data.preferredModel;
  const selectedProvider = modelSelection.providerId ? data.providers.find(provider => provider.id === modelSelection.providerId) : undefined;
  const selectedRemoteModel = selectedProvider?.models.find(model => model.id === modelSelection.modelId);
  const remoteReady = Boolean(selectedProvider?.verified && selectedProvider.hasApiKey && selectedRemoteModel?.contextLength);
  const chatReady = modelSelection.providerId ? remoteReady && !active?.providerSelectionRequired : nativeAvailable && data.runtime.phase === 'ready';
  const modelLabel = modelSelection.providerId ? selectedProvider ? `${selectedProvider.name} · ${modelSelection.modelId || 'Choose a model'}` : 'Provider deleted · choose another' : data.runtime.phase === 'ready' ? `Local · ${data.runtime.modelPath?.split(/[\\/]/).pop() || 'llama.cpp'}` : 'Local model';
  function navigate(nextPage: Page) {
    setError('');
    setPage(nextPage);
  }
  function newConversation() {
    switchDraft(null);
    selection.current++; setActiveId(null); setMessages([]); setPage('chat'); setError(''); setChatError(''); setLiveActivity(null); setRenaming(false);
    setAccessMode('ask');
    setSelectedConnectors(rememberedTools.current.sources); setSelectedTools(rememberedTools.current.tools);
  }
  async function changeTools(sources: string[], tools: ToolSelection[], mode: AccessMode = accessMode) {
    if (busy || toolSave.current) return;
    toolSave.current = true; setSavingTools(true); setError('');
    const conversationId = activeId;
    let savedConversation = false;
    let failure = '';
    try {
      if (conversationId) {
        await api.saveConversationTools(conversationId, { sources, tools, accessMode: mode });
        savedConversation = true;
      }
      if (nativeAvailable) {
        await api.saveRememberedTools({ sources, tools });
        rememberedTools.current = { sources, tools };
      }
    } catch (e) {
      failure = savedConversation
        ? `Your tools are saved for this conversation, but remembering them for new chats failed: ${errorMessage(e)}`
        : `Your tool selection was not saved: ${errorMessage(e)}`;
    }
    // Apply the choice once every persistence target for the visible
    // conversation holds it; a conversation save failure keeps the old state.
    if (savedConversation || !failure) {
      setSelectedConnectors(sources); setSelectedTools(tools); setAccessMode(mode);
    }
    if (failure) setError(failure);
    toolSave.current = false; setSavingTools(false);
  }
  async function saveModelSelection(next: ModelSelection) {
    if (busy) return;
    setError('');
    try {
      if (activeId) {
        await api.saveConversationModel(activeId, next);
        setData(current => ({ ...current, conversations: current.conversations.map(item => item.id === activeId ? { ...item, providerId: next.providerId, modelId: next.modelId || null, providerSelectionRequired: false } : item) }));
      } else {
        await api.savePreferredModel(next);
        setData(current => ({ ...current, preferredModel: next }));
      }
    } catch (e) { setError(errorMessage(e)); throw e; }
  }
  async function refreshProviders() {
    const next = normalizeBootstrap(await api.bootstrap());
    setData(next);
  }

  useEffect(() => { document.documentElement.dataset.theme = theme; localStorage.setItem('locallm-theme', theme); }, [theme]);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.bootstrap().then(value => {
      if (!disposed) {
        setData(normalizeBootstrap(value));
        // Startup restores the remembered selection for the initial new chat
        // while interaction is still blocked by `loading`.
        rememberedTools.current = value.rememberedTools ?? { sources: [], tools: [] };
        setSelectedConnectors(rememberedTools.current.sources);
        setSelectedTools(rememberedTools.current.tools);
      }
    }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setLoading(false); });
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
    switchDraft(id);
    setPage('chat'); setActiveId(id); setLoading(true); setError(''); setChatError(''); setLiveActivity(null); setRenaming(false);
    setMessages([]); setSelectedConnectors([]); setSelectedTools([]); setAccessMode('ask');
    try {
      const [next, tools] = await Promise.all([api.messages(id), api.conversationTools(id)]);
      if (version === selection.current) { setMessages(next); setSelectedConnectors(tools.sources); setSelectedTools(tools.tools); setAccessMode(tools.accessMode || 'ask'); }
    }
    catch (e) { if (version === selection.current) { setError(errorMessage(e)); setActiveId(null); switchDraft(null); setSelectedConnectors(rememberedTools.current.sources); setSelectedTools(rememberedTools.current.tools); } }
    finally { if (version === selection.current) setLoading(false); }
  }
  async function send(content: string) {
    if (busy || toolSave.current) throw new Error('Wait for the active operation before sending a message.');
    setGenerating(true); setChatError(''); setLiveActivity({ state: 'Preparing', activity: 'Preparing request…' });
    let id = activeId;
    const previousIds = new Set(messages.map(message => message.id));
    try {
      if (!id) {
        const conversation = await api.createConversation(); id = conversation.id;
        try { moveNewDraft(id); } catch (e) { setError(errorMessage(e)); }
        draftId.current = id;
        setActiveId(id); setData(current => ({ ...current, conversations: [conversation, ...current.conversations] }));
      }
      await api.saveConversationTools(id, { sources: selectedConnectors, tools: selectedTools, accessMode });
      const conversationId = id;
      setMessages(current => [...current, { id: 'pending-user', conversationId, role: 'user', content, reasoning: '', status: 'complete', createdAt: Date.now() }]);
      await api.sendMessage(conversationId, content, event => {
        if (event.context) setContextUsage({ conversationId, usage: event.context });
        if ('approval' in event) setApproval(event.approval || null);
        if (event.state || event.activity) {
          setLiveActivity({ state: event.state, activity: event.activity });
        }
        setMessages(current => {
          const existing = current.find(message => message.id === event.messageId);
          if (existing) return current.map(message => message.id === event.messageId ? { ...message, content: message.content + event.content, reasoning: message.reasoning + event.reasoning } : message);
          return [...current, { id: event.messageId, conversationId, role: 'assistant', content: event.content, reasoning: event.reasoning, status: 'streaming', createdAt: Date.now() }];
        });
      }, selectedConnectors, selectedTools);
    } catch (e) {
      setChatError(errorMessage(e));
      // Restore the draft only when persistence confirms the message was not accepted.
      // An inference failure can happen after the user message has already been saved.
      if (!id) throw e;
      let saved: Message[];
      try { saved = await api.messages(id); }
      catch {
        setChatError(`${errorMessage(e)} Could not verify whether your message was saved. Reopen the conversation before sending it again.`);
        return;
      }
      setMessages(saved);
      if (!saved.some(message => message.role === 'user' && !previousIds.has(message.id) && message.content === content.trim())) throw e;
    }
    finally {
      try { if (id) setMessages(await api.messages(id)); setData(normalizeBootstrap(await api.bootstrap())); }
      catch (e) { setChatError(errorMessage(e)); }
      setApproval(null);
      setGenerating(false);
      setLiveActivity(null);
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
    try { await api.deleteConversation(activeId); try { saveDraft(activeId, ''); } catch (e) { setError(errorMessage(e)); } setData(current => ({ ...current, conversations: current.conversations.filter(item => item.id !== activeId) })); newConversation(); }
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
    {!collapsed && <Sidebar page={page} onPage={navigate} conversations={data.conversations} activeId={activeId} onSelect={id => void selectConversation(id)} onNew={newConversation} busy={busy} search={search} onSearch={setSearch} theme={theme} onTheme={() => setTheme(theme === 'dark' ? 'light' : 'dark')} onCollapse={() => setCollapsed(true)} />}
    {approval && <ApprovalDialog key={approval.id} request={approval} onResolve={allow => api.resolveToolApproval(approval.id, allow)} />}
    <main className="workspace"><header className="workspace-header"><div>{collapsed && <button className="icon-button" aria-label="Expand sidebar" onClick={() => setCollapsed(false)}><PanelLeftOpen size={18} /></button>}{renaming && page === 'chat' ? <form className="rename-form" onSubmit={event => { event.preventDefault(); void rename(); }}><input autoFocus aria-label="Conversation title" maxLength={160} value={title} onChange={e => setTitle(e.target.value)} /><button className="icon-button" aria-label="Save title"><Check size={16} /></button><button type="button" className="icon-button" aria-label="Cancel rename" onClick={() => setRenaming(false)}><X size={16} /></button></form> : <span className="workspace-title">{page === 'chat' ? active?.title || 'New conversation' : page === 'models' ? 'Models & runtime' : page === 'connectors' ? 'Connectors' : page === 'execution' ? 'Execution' : page === 'memory' ? 'Memory' : page === 'plugins' ? 'Plugins' : 'Skills'}</span>}</div><div className="header-actions">{page === 'chat' && active && <><button className="icon-button" title="Rename conversation" aria-label="Rename conversation" disabled={busy} onClick={() => { setTitle(active.title); setRenaming(true); }}><Pencil size={15} /></button><button className="icon-button" title="Export conversation" aria-label="Export conversation" disabled={!messages.length || busy} onClick={() => void exportChat()}><Download size={15} /></button><button className="icon-button" title="Delete conversation" aria-label="Delete conversation" disabled={busy} onClick={() => void removeConversation()}><Trash2 size={15} /></button></>}<button className="model-selector" disabled={busy} onClick={() => navigate('models')}><span className={`status-dot ${chatReady ? 'ready' : ''}`} />{modelLabel}<ChevronDown size={13} /></button></div></header>
      {!nativeAvailable && <div className="preview-banner">Browser preview · Open the desktop application to load models and save conversations.</div>}
      {exportedPath && <div className="preview-banner" role="status">Saved conversation to {exportedPath}<button className="icon-button" aria-label="Dismiss export confirmation" onClick={() => setExportedPath('')}><X size={16} /></button></div>}
      {error && <div className="error-banner" role="alert"><span>{error}</span><button className="icon-button" aria-label="Dismiss error" onClick={() => setError('')}><X size={16} /></button></div>}
      {page === 'chat' ? <><Chat
        attachmentDrafts={attachmentDrafts}
        onAttachmentDraftsChange={setAttachmentDrafts}
        conversationKey={activeId ?? 'new'}
        composerTools={<><PresetPicker conversationId={activeId} disabled={busy} /><ToolPicker accessMode={accessMode} onAccessModeChange={mode => void changeTools(selectedConnectors, selectedTools, mode)} selectedTools={selectedTools} onToolsChange={tools => void changeTools(selectedConnectors, tools)} selected={selectedConnectors} onChange={sources => void changeTools(sources, selectedTools)} busy={busy} loading={loading} saving={savingTools} onNavigate={target => navigate(target)} /></>}
        contextUsage={contextUsage?.conversationId === activeId ? contextUsage.usage : undefined}
        modelLabel={modelLabel}
        selectionIssue={active?.providerSelectionRequired || (Boolean(modelSelection.providerId) && !remoteReady)}
        draft={draft}
        onDraftChange={changeDraft}
        messages={messages}
        generating={generating}
        ready={chatReady}
        loading={loading}
        disabled={savingTools || exporting}
        onSend={send}
        onCancel={() => { void api.cancelGeneration().catch(e => setChatError(errorMessage(e))); }}
        onConfigure={() => navigate('models')}
        liveActivity={liveActivity}
        providers={data.providers}
        onSelectModel={saveModelSelection}
        chatError={chatError}
        onDismissError={() => setChatError('')}
      /><Trajectory conversationId={activeId} /><TodosPanel conversationId={activeId} /><SessionsPanel conversationId={activeId} messages={messages} onForked={id => void selectConversation(id)} /></> : page === 'models' ? <Models config={data.config} preferences={data.preferences} runtime={data.runtime} providers={data.providers} selection={modelSelection} busy={!nativeAvailable || busy} onLoad={() => void modelAction(true)} onUnload={() => void modelAction(false)} onSaveSelection={saveModelSelection} onProvidersChanged={refreshProviders} onSaveConfig={async config => { try { await api.saveConfig(config); setData(current => ({ ...current, config })); } catch (e) { setError(errorMessage(e)); throw e; } }} onSavePreferences={async preferences => { try { await api.savePreferences(preferences); setData(current => ({ ...current, preferences })); } catch (e) { setError(errorMessage(e)); throw e; } }} /> : page === 'execution' ? <Execution /> : page === 'connectors' ? <Connectors /> : page === 'memory' ? <Memory /> : page === 'plugins' ? <Plugins /> : <Skills />}
    </main>
  </div>;
}
