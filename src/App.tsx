import { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, Download, PanelLeftOpen, Pencil, Trash2, X, ArrowLeft, ArrowRight, Folder, MoreHorizontal, Cpu, Plug, BookOpen, Wrench, Terminal, Settings } from 'lucide-react';

import { confirm, save } from '@tauri-apps/plugin-dialog';

import { api, errorMessage, nativeAvailable } from './lib/api';

import { defaultRuntimeConfig, type Bootstrap, type Message, type ToolApproval, type ToolSelection, type AccessMode, type ContextUsage, type ModelSelection, type RememberedTools } from './lib/types';

import { Sidebar, type Page } from './components/Sidebar';

import type { Attachment } from './lib/attachments';

import { Chat } from './components/Chat';

import { Models } from './components/Models';

import { DownloadActivity } from './components/DownloadActivity';

import { Skills } from './components/Skills';

import { Execution } from './components/Execution';

import { Connectors } from './components/Connectors';

import { ApprovalDialog, ToolsSettings, PermissionSelector } from './components/ToolControls';

import { Trajectory } from './components/Trajectory';

import { WindowControls } from './components/WindowControls';
import { WorkspacePanel } from './components/WorkspacePanel';

import type { WorkspaceIndex } from './lib/types';

import './styles.css';

import './workspace.css';

import './models.css';

import { loadDraft, saveDraft, moveNewDraft } from './lib/drafts';



const initialData: Bootstrap = {

  conversations: [], config: defaultRuntimeConfig,

  preferences: { runtimePath: '', modelPath: '', projectorPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: 'You are a helpful local assistant. Be clear and accurate. If you do not know something, say so.' },

  runtime: { phase: 'stopped', message: 'No model loaded', modelPath: null },

  rememberedTools: { sources: [], tools: [] },

  providers: [], preferredModel: { providerId: null, modelId: '' },

};

function normalizeBootstrap(value: Bootstrap): Bootstrap {

  return { ...value, providers: value.providers ?? [], preferredModel: value.preferredModel ?? { providerId: null, modelId: '' }, rememberedTools: value.rememberedTools ?? { sources: [], tools: [] } };

}

function sourcesForNewConversation(sources: string[], projectId: string | null) {

  return projectId ? sources : sources.filter(source => source !== '__workspace' && source !== '__execution');

}



export default function App() {

  const [workspaceIndex, setWorkspaceIndex] = useState<WorkspaceIndex>({ projects: [], tasks: {} });

  const [projectId, setProjectId] = useState<string | null>(null);

  const [commandBusy, setCommandBusy] = useState(false);

  const [navigation, setNavigation] = useState<{ ids: string[]; cursor: number }>({ ids: [], cursor: -1 });

  const [hitId, setHitId] = useState<string | null>(null);
  const [gitBranch, setGitBranch] = useState('feature/locallm');

  useEffect(() => {
    if (nativeAvailable && api.workspaceGit) {
      api.workspaceGit().then(g => { if (g?.branch) setGitBranch(g.branch); }).catch(() => {});
    }
  }, []);

  useEffect(() => { if (nativeAvailable && api.workspaceIndex) api.workspaceIndex().then(setWorkspaceIndex).catch(e => setError(errorMessage(e))); }, []);

  async function selectProject(id: string | null) {

    try {

      const project = workspaceIndex.projects.find(p => p.id === id);

      await api.setWorkspace(project?.path ?? '');

      if (activeId) setWorkspaceIndex(await api.saveTaskMeta(activeId, { ...(workspaceIndex.tasks[activeId] ?? { archived: false, pinned: false }), projectId: id }));

      setProjectId(id);

    } catch (e) { setError(errorMessage(e)); }

  }

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

  // Composer "Plan mode" toggle: on send, a tool-free planning pass runs,
  // then the ordinary tool loop implements the plan (multi-hop; no early
  // research stop). Per-session, defaults off.

  const [planMode, setPlanMode] = useState(false);

  const [selectedTools, setSelectedTools] = useState<ToolSelection[]>([]);

  // Last explicitly chosen selection for future chats; restored at startup and

  // applied by newConversation. Existing conversations never read from it.

  const rememberedTools = useRef<RememberedTools>({ sources: [], tools: [] });

  const [approval, setApproval] = useState<ToolApproval | null>(null);

  const [data, setData] = useState(initialData);

  const [page, setPage] = useState<Page>('chat');

  const [requestedModel, setRequestedModel] = useState<string>();

  const [activeId, setActiveId] = useState<string | null>(null);

  const [messages, setMessages] = useState<Message[]>([]);

  const [contextUsage, setContextUsage] = useState<{ conversationId: string; usage: ContextUsage } | null>(null);

  const [loading, setLoading] = useState(nativeAvailable);

  useEffect(() => {

    if (loading || !hitId) return;

    const element = document.querySelector(`[data-message-id="${CSS.escape(hitId)}"]`);

    if (element) { element.scrollIntoView({ block: 'center' }); element.classList.add('search-highlight'); setHitId(null); }

  }, [messages, loading, hitId]);

  const [generating, setGenerating] = useState(false);

  const [liveActivity, setLiveActivity] = useState<{ state?: string; activity?: string } | null>(null);

  const [chatError, setChatError] = useState('');

  const [chatNotice, setChatNotice] = useState('');

  const [modelBusy, setModelBusy] = useState(false);

  const [savingTools, setSavingTools] = useState(false);

  const [exporting, setExporting] = useState(false);

  const [exportedPath, setExportedPath] = useState('');

  const [preset, setPreset] = useState<string>('standard');

  const toolSave = useRef(false);

  const [error, setError] = useState('');

  const [search, setSearch] = useState('');

  const [collapsed, setCollapsed] = useState(false);

  const [renaming, setRenaming] = useState(false);

  const [title, setTitle] = useState('');

  const [theme, setTheme] = useState(() => localStorage.getItem('locallm-theme') || 'dark');

  const selection = useRef(0);

  const active = data.conversations.find(item => item.id === activeId);

  const busy = generating || modelBusy || loading || savingTools || exporting || commandBusy;

  const modelSelection: ModelSelection = active ? { providerId: active.providerId, modelId: active.modelId ?? '' } : data.preferredModel;

  const selectedProvider = modelSelection.providerId ? data.providers.find(provider => provider.id === modelSelection.providerId) : undefined;

  const selectedRemoteModel = selectedProvider?.models.find(model => model.id === modelSelection.modelId);

  // Verified loopback engines (llama.cpp servers, FreeToken) run without an API key.

  const providerLoopback = Boolean(selectedProvider && (selectedProvider.baseUrl.includes('localhost') || selectedProvider.baseUrl.includes('127.0.0.1') || selectedProvider.baseUrl.includes('::1')));

  const remoteReady = Boolean(selectedProvider?.verified && (selectedProvider.hasApiKey || providerLoopback) && selectedRemoteModel?.contextLength);

  const chatReady = modelSelection.providerId ? remoteReady && !active?.providerSelectionRequired : nativeAvailable && data.runtime.phase === 'ready';

  const modelLabel = modelSelection.providerId ? selectedProvider ? `${selectedProvider.name} · ${modelSelection.modelId || 'Choose a model'}` : 'Provider deleted · choose another' : data.runtime.phase === 'ready' ? `Local · ${data.runtime.modelPath?.split(/[\\/]/).pop() || 'llama.cpp'}` : 'Local model';

  function navigate(nextPage: Page) {

    setError('');

    setPage(nextPage);

  }

  async function newConversation(targetProjectId: string | null = null) {

    setChatNotice('');

    switchDraft(null);

    selection.current++; setActiveId(null); setMessages([]); setPage('chat'); setError(''); setChatError(''); setLiveActivity(null); setRenaming(false);

    setAccessMode(rememberedTools.current.accessMode ?? 'ask');

    setPlanMode(false);

    setPreset('standard');

    setSelectedConnectors(sourcesForNewConversation(rememberedTools.current.sources, targetProjectId));
    setSelectedTools(rememberedTools.current.tools);

    setProjectId(targetProjectId);

    try {
      if (targetProjectId) {
        const project = workspaceIndex.projects.find(p => p.id === targetProjectId);
        if (project?.path && api.setWorkspace) {
          await api.setWorkspace(project.path);
        }
      } else if (api.setWorkspace) {
        await api.setWorkspace('');
      }
    } catch (e) {
      setError(errorMessage(e));
    }

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

        await api.saveRememberedTools({ sources, tools, accessMode: mode });

        rememberedTools.current = { sources, tools, accessMode: mode };

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

  async function changePreset(nextPreset: string) {

    if (busy) return;

    setPreset(nextPreset);

    if (activeId && api.setPreset) {

      try {

        await api.setPreset(activeId, nextPreset);

      } catch (e) {

        setError(errorMessage(e));

      }

    }

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

    } catch (e) { setError(errorMessage(e)); }

  }

  async function refreshProviders() {

    try {

      const providers = await api.listProviders();

      setData(current => ({ ...current, providers }));

    } catch (e) {

      setError(errorMessage(e));

    }

  }

  useEffect(() => {

    const closeMenus = (event: MouseEvent | KeyboardEvent) => {

      if (event instanceof KeyboardEvent && event.key !== 'Escape') return;

      document.querySelectorAll<HTMLDetailsElement>('.app-menu[open]').forEach(menu => {

        if (event instanceof KeyboardEvent || !menu.contains(event.target as Node)) {

          menu.open = false;

          if (event instanceof KeyboardEvent) menu.querySelector<HTMLElement>('summary')?.focus();

        }

      });

    };

    document.addEventListener('click', closeMenus);

    document.addEventListener('keydown', closeMenus);

    return () => { document.removeEventListener('click', closeMenus); document.removeEventListener('keydown', closeMenus); };

  }, []);

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

        setSelectedConnectors(sourcesForNewConversation(rememberedTools.current.sources, null));

        setSelectedTools(rememberedTools.current.tools);
        setAccessMode(rememberedTools.current.accessMode ?? 'ask');

      }

    }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setLoading(false); });

    return () => { disposed = true; };

  }, []);

  useEffect(() => {

    function keydown(event: KeyboardEvent) {

      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {

        event.preventDefault(); setCollapsed(false); setTimeout(() => document.querySelector<HTMLInputElement>('[aria-label="Search conversations"]')?.focus(), 0);

      }

      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'n') {

        event.preventDefault(); if (!busy) newConversation();

      }

    }

    window.addEventListener('keydown', keydown); return () => window.removeEventListener('keydown', keydown);

  }, [busy]);

  async function selectConversation(id: string, fromHistory = false) {

    if (busy || toolSave.current) return;

    setChatNotice('');

    const version = ++selection.current;

    switchDraft(id);

    setPage('chat'); setActiveId(id); setLoading(true); setError(''); setChatError(''); setLiveActivity(null); setRenaming(false);

    setMessages([]); setSelectedConnectors([]); setSelectedTools([]); setAccessMode('ask'); setPlanMode(false); setPreset('standard');

    try {

      const nextProject = workspaceIndex.tasks[id]?.projectId ?? null;

      const project = workspaceIndex.projects.find(p => p.id === nextProject);

      if (project) await api.setWorkspace(project.path);

      setProjectId(nextProject);

      const [next, tools, conversationPreset] = await Promise.all([

        api.messages(id),

        api.conversationTools(id),

        api.getPreset ? api.getPreset(id).catch(() => null) : Promise.resolve(null),

      ]);

      if (version === selection.current) {

        if (!fromHistory) setNavigation(n => ({ ids: [...n.ids.slice(0, n.cursor + 1), id], cursor: n.cursor + 1 }));

        setMessages(next);

        setSelectedConnectors(tools.sources);

        setSelectedTools(tools.tools);

        setAccessMode(tools.accessMode || 'ask');

        if (conversationPreset?.id) setPreset(conversationPreset.id);

      }

    }

    catch (e) { if (version === selection.current) { setError(errorMessage(e)); setActiveId(null); switchDraft(null); setSelectedConnectors(sourcesForNewConversation(rememberedTools.current.sources, null)); setSelectedTools(rememberedTools.current.tools); setAccessMode(rememberedTools.current.accessMode ?? 'ask'); } }

    finally { if (version === selection.current) setLoading(false); }

  }

  async function send(content: string) {

    if (busy || toolSave.current) throw new Error('Wait for the active operation before sending a message.');

    setChatNotice('');

    setGenerating(true); setChatError(''); setLiveActivity({ state: 'Preparing', activity: 'Preparing request…' });

    let id = activeId;

    const previousIds = new Set(messages.map(message => message.id));

    try {

      if (!id) {

        const conversation = await api.createConversation(); id = conversation.id;

        if (projectId) setWorkspaceIndex(await api.saveTaskMeta(id, { projectId, archived: false, pinned: false }));

        try { moveNewDraft(id); } catch (e) { setError(errorMessage(e)); }

        draftId.current = id;

        setActiveId(id); setData(current => ({ ...current, conversations: [conversation, ...current.conversations] }));

        // Persist the profile picked for this new chat before the turn is

        // assembled, so the backend renders the same payload the user chose.

        if (preset !== 'standard' && api.setPreset) {

          try { await api.setPreset(id, preset); } catch (e) { setError(errorMessage(e)); }

        }

      }

      await api.saveConversationTools(id, { sources: selectedConnectors, tools: selectedTools, accessMode });

      const conversationId = id;

      setMessages(current => [...current, { id: 'pending-user', conversationId, role: 'user', content, reasoning: '', status: 'complete', createdAt: Date.now() }]);

      await api.sendMessage(conversationId, content, event => {

        if (event.notice) { setChatNotice(event.notice); return; }

        if (event.context) setContextUsage({ conversationId, usage: event.context });

        if ('approval' in event) setApproval(event.approval || null);

        if (event.state || event.activity) {

          setLiveActivity({ state: event.state, activity: event.activity });

        }

        if (event.toolStream) {

          const stream = event.toolStream;

          const toolMsgId = `tool-${stream.toolCallId}`;

          if (stream.type === 'started') {

            setMessages(current => {

              if (current.some(m => m.id === toolMsgId)) return current;

              return [

                ...current,

                {

                  id: toolMsgId,

                  conversationId,

                  role: 'tool',

                  content: JSON.stringify({

                    request: {

                      name: stream.toolName,

                      arguments: {

                        command: stream.command,

                        language: stream.language,

                        cwd: stream.cwd,

                      },

                      decision: 'allowed',

                    },

                    result: {

                      stdout: '',

                      stderr: '',

                      exitCode: null,

                    },

                  }),

                  reasoning: '',

                  status: 'streaming',

                  createdAt: Date.now(),

                },

              ];

            });

            return;

          }

          if (stream.type === 'outputChunk') {

            setMessages(current =>

              current.map(m => {

                if (m.id !== toolMsgId) return m;

                let record: Record<string, unknown> = {};

                try { record = JSON.parse(m.content); } catch { /* ignore */ }

                const res = (record.result && typeof record.result === 'object' && !Array.isArray(record.result))

                  ? { ...(record.result as Record<string, unknown>) }

                  : {};

                const streamKey = stream.stream === 'stderr' ? 'stderr' : 'stdout';

                const prevText = typeof res[streamKey] === 'string' ? (res[streamKey] as string) : '';

                res[streamKey] = prevText + stream.chunk;

                record.result = res;

                return { ...m, content: JSON.stringify(record) };

              })

            );

            return;

          }

          if (stream.type === 'finished') {

            setMessages(current =>

              current.map(m => {

                if (m.id !== toolMsgId) return m;

                let record: Record<string, unknown> = {};

                try { record = JSON.parse(m.content); } catch { /* ignore */ }

                const res = (record.result && typeof record.result === 'object' && !Array.isArray(record.result))

                  ? { ...(record.result as Record<string, unknown>) }

                  : {};

                res.exitCode = stream.exitCode;

                res.durationMs = stream.durationMs;

                if (stream.error) res.error = stream.error;

                record.result = res;

                const isErr = (stream.exitCode !== null && stream.exitCode !== undefined && stream.exitCode !== 0) || Boolean(stream.error);

                return { ...m, content: JSON.stringify(record), status: isErr ? 'error' : 'complete' };

              })

            );

            return;

          }

        }

        setMessages(current => {

          const existing = current.find(message => message.id === event.messageId);

          if (existing) return current.map(message => message.id === event.messageId ? { ...message, content: message.content + event.content, reasoning: message.reasoning + event.reasoning } : message);

          return [...current, { id: event.messageId, conversationId, role: 'assistant', content: event.content, reasoning: event.reasoning, status: 'streaming', createdAt: Date.now() }];

        });

      }, selectedConnectors, selectedTools, planMode);

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

      // Always rethrow so Chat.tsx can restore the draft. The error is already

      // displayed via setChatError above; rethrowing ensures the draft and

      // attachment state are properly restored by the caller's catch block.

      if (!saved.some(message => message.role === 'user' && !previousIds.has(message.id) && message.content === content.trim())) throw e;

      // Even when the user message was saved, rethrow so the caller knows

      // inference failed and can offer retry.

      throw e;

    }

    finally {

      try { if (id) setMessages(await api.messages(id)); setData(normalizeBootstrap(await api.bootstrap())); }

      catch (e) {

        // Finalize any messages stuck in 'streaming' status so the UI doesn't

        // show perpetual spinners when the backend died mid-turn.

        setMessages(current => current.map(m =>

          m.status === 'streaming' ? { ...m, status: 'error' } : m

        ));

        setChatError(errorMessage(e));

      }

      setApproval(null);

      setGenerating(false);

      setLiveActivity(null);

    }

  }

  async function modelAction(load: boolean) {

    setModelBusy(true); setError('');

    try {

      const runtime = await (load ? api.loadModel() : api.unloadModel());

      setData(current => ({ ...current, runtime }));

      // Loading may apply a model-specific runtime profile (for example, the

      // Prism server required by Bonsai). Refresh the persisted preferences so

      // the settings page shows the executable actually used by the server.

      if (load) {

        try {

          setData(normalizeBootstrap(await api.bootstrap()));

        } catch (refreshError) {

          // The model is already running; a bootstrap refresh failure should

          // not turn a successful load into a false runtime error.

          setError(`Model loaded, but settings could not be refreshed: ${errorMessage(refreshError)}`);

        }

      }

    }

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

  const activeProject = workspaceIndex.projects.find(p => p.id === projectId);
  const activeProjectName = activeProject?.name;

  return <div className={`app ${collapsed ? 'sidebar-collapsed' : ''}`}>

    <div className="app-menubar" aria-label="Application toolbar">

      <div className="menubar-nav-group">
        <button
          className="icon-button menubar-nav-btn menubar-sidebar-toggle"
          aria-label="Toggle sidebar"
          title="Toggle sidebar"
          onClick={() => setCollapsed(!collapsed)}
        >
          <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5">
            <rect x="2" y="2" width="12" height="12" rx="2" />
            <line x1="6" y1="2" x2="6" y2="14" />
          </svg>
        </button>
        <button
          className="icon-button menubar-nav-btn"
          aria-label="Back"
          disabled={busy || navigation.cursor <= 0}
          onClick={() => {
            const cursor = navigation.cursor - 1;
            setNavigation(n => ({ ...n, cursor }));
            void selectConversation(navigation.ids[cursor], true);
          }}
        >
          <ArrowLeft size={16} />
        </button>
        <button
          className="icon-button menubar-nav-btn"
          aria-label="Forward"
          disabled={busy || navigation.cursor >= navigation.ids.length - 1}
          onClick={() => {
            const cursor = navigation.cursor + 1;
            setNavigation(n => ({ ...n, cursor }));
            void selectConversation(navigation.ids[cursor], true);
          }}
        >
          <ArrowRight size={16} />
        </button>
      </div>

      <details className="app-menu" name="application-menu"><summary>File</summary><div><button disabled={busy} onClick={event => { newConversation(); event.currentTarget.closest('details')?.removeAttribute('open'); }}>New task <kbd>Ctrl+N</kbd></button><button disabled={!active || busy} onClick={() => void exportChat()}>Save a copy…</button></div></details>

      <details className="app-menu" name="application-menu"><summary>Edit</summary><div><button disabled={!active || busy} onClick={event => { if (active) { setTitle(active.title); setRenaming(true); } event.currentTarget.closest('details')?.removeAttribute('open'); }}>Rename task</button><button onClick={event => { setCollapsed(false); setTimeout(() => document.querySelector<HTMLInputElement>('[aria-label="Search conversations"]')?.focus(), 0); event.currentTarget.closest('details')?.removeAttribute('open'); }}>Search tasks <kbd>Ctrl+K</kbd></button></div></details>

      <details className="app-menu" name="application-menu"><summary>View</summary><div><button onClick={() => setCollapsed(!collapsed)}>Toggle sidebar</button><button onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}>Switch to {theme === 'dark' ? 'light' : 'dark'} theme</button><button onClick={event => { navigate('models'); event.currentTarget.closest('details')?.removeAttribute('open'); }}>Models &amp; runtime</button><button onClick={event => { navigate('connectors'); event.currentTarget.closest('details')?.removeAttribute('open'); }}>Connectors</button><button onClick={event => { navigate('skills'); event.currentTarget.closest('details')?.removeAttribute('open'); }}>Skills</button><button onClick={event => { navigate('tools'); event.currentTarget.closest('details')?.removeAttribute('open'); }}>Tools &amp; permissions</button><button onClick={event => { navigate('execution'); event.currentTarget.closest('details')?.removeAttribute('open'); }}>Execution</button></div></details>

      <details className="app-menu" name="application-menu"><summary>Help</summary><div className="shortcut-help"><strong>Workspace shortcuts</strong><span>Review <kbd>Ctrl+Shift+G</kbd></span><span>Terminal <kbd>Ctrl+`</kbd></span><span>Browser <kbd>Ctrl+T</kbd></span><span>Files <kbd>Ctrl+P</kbd></span></div></details>

      <div className="titlebar-drag-region" data-tauri-drag-region aria-hidden="true" />

      <WindowControls onError={setError} />
    </div>

    {!collapsed && <Sidebar workspace={workspaceIndex} onWorkspace={setWorkspaceIndex} projectId={projectId} onProject={id => void selectProject(id)} onHit={(id, messageId) => { setHitId(messageId); void selectConversation(id); }} page={page} onPage={navigate} conversations={data.conversations} activeId={activeId} onSelect={id => void selectConversation(id)} onNew={targetProjectId => void newConversation(targetProjectId)} busy={busy} search={search} onSearch={setSearch} theme={theme} onTheme={() => setTheme(theme === 'dark' ? 'light' : 'dark')} onCollapse={() => setCollapsed(true)} />}

    {approval && <ApprovalDialog key={approval.id} request={approval} onResolve={allow => api.resolveToolApproval(approval.id, allow)} onResolveAskUser={(choice) => api.resolveAskUser(approval.id, choice)} />}

    <main className="workspace"><header className={`workspace-header ${page === 'chat' && !messages.length && !renaming ? 'workspace-header-minimal' : ''}`}><div className="workspace-lead">{collapsed && <button className="icon-button" aria-label="Expand sidebar" onClick={() => setCollapsed(false)}><PanelLeftOpen size={18} /></button>}{page === 'chat' ? <><button className="icon-button" aria-label="Back" disabled={busy || navigation.cursor <= 0} onClick={() => { const cursor = navigation.cursor - 1; setNavigation(n => ({ ...n, cursor })); void selectConversation(navigation.ids[cursor], true); }}><ArrowLeft size={17} /></button><button className="icon-button" aria-label="Forward" disabled={busy || navigation.cursor >= navigation.ids.length - 1} onClick={() => { const cursor = navigation.cursor + 1; setNavigation(n => ({ ...n, cursor })); void selectConversation(navigation.ids[cursor], true); }}><ArrowRight size={17} /></button><Folder className="task-folder-icon" size={18} />{renaming ? <form className="rename-form" onSubmit={event => { event.preventDefault(); void rename(); }}><input autoFocus aria-label="Conversation title" maxLength={160} value={title} onChange={e => setTitle(e.target.value)} /><button className="icon-button" aria-label="Save title"><Check size={16} /></button><button type="button" className="icon-button" aria-label="Cancel rename" onClick={() => setRenaming(false)}><X size={16} /></button></form> : <span className="workspace-title">{active?.title || 'New conversation'}</span>}</> : <><Settings className="task-folder-icon" size={18} /><span className="workspace-title">{page === 'models' ? 'Models & runtime' : page === 'connectors' ? 'Connectors' : page === 'execution' ? 'Execution' : page === 'tools' ? 'Tools' : 'Skills'}</span></>}</div><div className="header-actions">{page === 'chat' && workspaceIndex.projects.length > 0 && <select aria-label="Task project" value={projectId ?? ''} disabled={busy} onChange={e => void selectProject(e.target.value || null)}><option value="">No project</option>{workspaceIndex.projects.map(p => <option value={p.id} key={p.id}>{p.name}</option>)}</select>}{page === 'chat' && active && <details className="app-menu task-menu" name="application-menu"><summary aria-label="Task actions"><MoreHorizontal size={19} /></summary><div><button className="icon-button" title="Rename conversation" aria-label="Rename conversation" disabled={busy} onClick={() => { setTitle(active.title); setRenaming(true); }}><Pencil size={15} /></button><button className="icon-button" title="Export conversation" aria-label="Export conversation" disabled={!messages.length || busy} onClick={() => void exportChat()}><Download size={15} /></button><button className="icon-button" title="Delete conversation" aria-label="Delete conversation" disabled={busy} onClick={() => void removeConversation()}><Trash2 size={15} /></button></div></details>}{page === 'chat' && <div className="workspace-layout-controls"><button className="icon-button workspace-layout-btn" aria-label="Toggle split view" title="Toggle split view" onClick={() => setCollapsed(!collapsed)}><svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5"><rect x="2" y="2" width="12" height="12" rx="2" /><line x1="8" y1="2" x2="8" y2="14" /></svg></button><button className="icon-button workspace-layout-btn" aria-label="Toggle panel" title="Toggle panel" onClick={() => { const toggleBtn = document.querySelector<HTMLButtonElement>('[aria-label="Toggle files panel"]'); toggleBtn?.click(); }}><svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5"><rect x="2" y="2" width="12" height="12" rx="2" /><line x1="11" y1="2" x2="11" y2="14" /></svg></button></div>}{page === 'chat' && <WorkspacePanel busy={busy} onBusy={setCommandBusy} conversationId={activeId} revision={`${projectId}:${activeId}`} />}<button className="model-selector" disabled={busy} onClick={() => navigate('models')}><span className={`status-dot ${chatReady ? 'ready' : ''}`} />{modelLabel}<ChevronDown size={13} /></button></div></header><DownloadActivity />

      {page !== 'chat' && (
        <nav className="settings-nav-bar" aria-label="Settings navigation">
          <div className="settings-nav-tabs">
            <button
              className={`settings-nav-tab ${page === 'models' ? 'active' : ''}`}
              aria-label="Settings: Models & runtime"
              onClick={() => navigate('models')}
              title="Models & runtime (Local GGUF, llama.cpp, API keys, Proxies, Subscriptions)"
            >
              <Cpu size={14} />
              <span>Models &amp; runtime</span>
            </button>
            <button
              className={`settings-nav-tab ${page === 'connectors' ? 'active' : ''}`}
              aria-label="Settings: Connectors"
              onClick={() => navigate('connectors')}
              title="Connectors (MCP servers, external tools)"
            >
              <Plug size={14} />
              <span>Connectors</span>
            </button>
            <button
              className={`settings-nav-tab ${page === 'skills' ? 'active' : ''}`}
              aria-label="Settings: Skills"
              onClick={() => navigate('skills')}
              title="Skills (Reusable workflow instructions & scripts)"
            >
              <BookOpen size={14} />
              <span>Skills</span>
            </button>
            <button
              className={`settings-nav-tab ${page === 'tools' ? 'active' : ''}`}
              aria-label="Settings: Tools"
              onClick={() => navigate('tools')}
              title="Tools & permissions"
            >
              <Wrench size={14} />
              <span>Tools</span>
            </button>
            <button
              className={`settings-nav-tab ${page === 'execution' ? 'active' : ''}`}
              aria-label="Settings: Execution"
              onClick={() => navigate('execution')}
              title="Execution (Interpreters, Web Search)"
            >
              <Terminal size={14} />
              <span>Execution</span>
            </button>
          </div>
          <button
            className="settings-nav-back"
            aria-label="Close settings"
            onClick={() => navigate('chat')}
            title="Close settings and return to conversation"
          >
            <X size={14} />
            <span>Close</span>
          </button>
        </nav>
      )}

      {!nativeAvailable && <div className="preview-banner">Browser preview · Open the desktop application to load models and save conversations.</div>}

      {exportedPath && <div className="preview-banner" role="status">Saved conversation to {exportedPath}<button className="icon-button" aria-label="Dismiss export confirmation" onClick={() => setExportedPath('')}><X size={16} /></button></div>}

      {error && <div className="error-banner" role="alert"><span>{error}</span><button className="icon-button" aria-label="Dismiss error" onClick={() => setError('')}><X size={16} /></button></div>}

      {page === 'chat' ? <><Chat

        projectName={activeProjectName}

        gitBranch={activeProject ? gitBranch : undefined}

        attachmentDrafts={attachmentDrafts}

        onAttachmentDraftsChange={setAttachmentDrafts}

        conversationKey={activeId ?? 'new'}

        composerTools={<PermissionSelector accessMode={accessMode} onAccessModeChange={mode => void changeTools(selectedConnectors, selectedTools, mode)} busy={busy} nativeAvailable={nativeAvailable} />}

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

        preset={preset}

        connectorIds={selectedConnectors}

        connectorTools={selectedTools}

        onSend={send}

        onCancel={() => { void api.cancelGeneration().catch(e => setChatError(errorMessage(e))); }}

        onConfigure={() => navigate('models')}

        planMode={planMode}

        onPlanModeChange={setPlanMode}

        onConfigureLocalModel={filename => { setRequestedModel(filename); navigate('models'); }}

        liveActivity={liveActivity}

        providers={data.providers}

        onSelectModel={saveModelSelection}

        chatError={chatError}

        chatNotice={chatNotice}

        onDismissError={() => setChatError('')}

      /><Trajectory

        conversationId={activeId}

        onFork={async newId => {

          try {

            const refreshed = normalizeBootstrap(await api.bootstrap());

            setData(refreshed);

            await selectConversation(newId);

          } catch (e) {

            setError(errorMessage(e));

          }

        }}

      /></> : page === 'tools' ? <ToolsSettings accessMode={accessMode} onAccessModeChange={mode => void changeTools(selectedConnectors, selectedTools, mode)} preset={preset} onPresetChange={changePreset} selectedTools={selectedTools} onToolsChange={tools => void changeTools(selectedConnectors, tools)} selected={selectedConnectors} onChange={sources => void changeTools(sources, selectedTools)} scopeLabel={active?.title} onBack={() => navigate('chat')} busy={busy} loading={loading} saving={savingTools} onNavigate={target => navigate(target)} /> : page === 'models' ? <Models onRefresh={async () => { setData(normalizeBootstrap(await api.bootstrap())); }} requestedModel={requestedModel} onRequestedModelHandled={() => setRequestedModel(undefined)} config={data.config} preferences={data.preferences} runtime={data.runtime} providers={data.providers} selection={modelSelection} busy={!nativeAvailable || busy} onLoad={() => void modelAction(true)} onUnload={() => void modelAction(false)} onSaveSelection={saveModelSelection} onProvidersChanged={refreshProviders} onSaveConfig={async config => { try { await api.saveConfig(config); setData(normalizeBootstrap(await api.bootstrap())); } catch (e) { setError(errorMessage(e)); throw e; } }} onSavePreferences={async preferences => { try { await api.savePreferences(preferences); setData(normalizeBootstrap(await api.bootstrap())); } catch (e) { setError(errorMessage(e)); throw e; } }} /> : page === 'execution' ? <Execution /> : page === 'connectors' ? <Connectors /> : <Skills />}

    </main>

  </div>;

}
