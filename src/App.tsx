import { lazy, Suspense, useCallback, useEffect, useRef, useState } from 'react';
import { Check, Download, PanelLeftOpen, Pencil, Trash2, X, ArrowLeft, ArrowRight, Folder, MoreHorizontal } from 'lucide-react';

import { confirm, save } from '@tauri-apps/plugin-dialog';
import { listen } from '@tauri-apps/api/event';

import { api, errorMessage, nativeAvailable } from './lib/api';

import { defaultRuntimeConfig, type Bootstrap, type Message, type ToolApproval, type ToolSelection, type AccessMode, type InteractionMode, type ContextUsage, type ModelSelection, type RememberedTools } from './lib/types';

import { Sidebar, type Page } from './components/Sidebar';

import type { Attachment } from './lib/attachments';

import { Chat } from './components/Chat';
import { localInferenceProvider } from './lib/providerCapabilities';
import { useRuntimeStatusPoll } from './lib/runtimeStatusPoll';


import { DownloadActivity } from './components/DownloadActivity';




import { ApprovalDialog, ToolsSettings, PermissionSelector } from './components/ToolControls';

import { Trajectory } from './components/Trajectory';

import { WindowControls } from './components/WindowControls';
import { WorkspacePanel } from './components/WorkspacePanel';
import { AppearanceSettings, BrowserSettings, DataSettings, GeneralSettings, SettingsShell, ShortcutSettings } from './components/Settings';
import { applyAppearance, loadAppearance, saveAppearance } from './lib/appearance';

import type { Task, TaskWorkspace, WorkspaceIndex } from './lib/types';

import { allProjects, conversationProjectId, findProject } from './lib/projects';

import './styles.css';

import './workspace.css';

import './models.css';

import './settings.css';

import { loadDraft, saveDraft, moveNewDraft } from './lib/drafts';

const Models = lazy(() => import('./components/Models').then(module => ({ default: module.Models })));
const Skills = lazy(() => import('./components/Skills').then(module => ({ default: module.Skills })));
const Execution = lazy(() => import('./components/Execution').then(module => ({ default: module.Execution })));
const Documents = lazy(() => import('./components/Documents').then(module => ({ default: module.Documents })));
const Connectors = lazy(() => import('./components/Connectors').then(module => ({ default: module.Connectors })));



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

function sourcesForNewConversation(sources: string[], hasWorkspace: boolean) {

  return hasWorkspace ? sources : sources.filter(source => source !== '__workspace' && source !== '__execution');

}



export default function App() {

  const [workspaceIndex, setWorkspaceIndex] = useState<WorkspaceIndex>({ projects: [], tasks: {} });

  const [projectId, setProjectId] = useState<string | null>(null);

  const [navigation, setNavigation] = useState<{ ids: string[]; cursor: number }>({ ids: [], cursor: -1 });

  const [hitId, setHitId] = useState<string | null>(null);

  useEffect(() => { if (nativeAvailable && api.workspaceIndex) api.workspaceIndex().then(setWorkspaceIndex).catch(e => setError(errorMessage(e))); }, []);

  async function selectProject(id: string | null) {

    const target = id || null;

    try {

      const project = findProject(workspaceIndex, target);

      // Same lock as above: a live run keeps the workspace pinned to its own
      // task, but project selection should not be blocked by it.
      if (nativeAvailable && api.setWorkspace) {

        try { await api.setWorkspace(project?.path ?? ''); }

        catch (e) { setError(errorMessage(e)); }

      }

      if (activeId) setWorkspaceIndex(await api.saveTaskMeta(activeId, { ...(workspaceIndex.tasks[activeId] ?? { archived: false, pinned: false }), projectId: target }));

      setProjectId(target);

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

  // Ask/Plan/Act: persisted per conversation like the permission choice; the
  // last pick is remembered as the default for new chats. Old conversations
  // without a stored mode default to Act.

  const [interactionMode, setInteractionMode] = useState<InteractionMode>('act');

  // A completed Plan-mode turn sets this; the composer then offers
  // "Implement plan", which sends a follow-up turn in Act mode.

  const [planReady, setPlanReady] = useState(false);

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

  // A task the scheduler started in the open chat runs without a Channel —
  // track it so the composer still offers Queue/Steer and the Stop button
  // reaches its cancel signal.
  const [runningTask, setRunningTask] = useState<Task | null>(null);
  useEffect(() => {
    if (!activeId || !nativeAvailable) { setRunningTask(null); return; }
    let alive = true;
    const check = async () => {
      try {
        const busy = await api.tasksList(['running', 'waiting_approval', 'waiting_resource', 'paused']);
        if (alive) setRunningTask(busy.find(task => task.conversationId === activeId) ?? null);
      } catch { /* inbox is a signal, not a hard dependency */ }
    };
    void check();
    const timer = window.setInterval(() => void check(), 2000);
    let unlisten: (() => void) | undefined;
    void listen('task-event', event => {
      const payload = event.payload as { conversationId?: string };
      if (payload?.conversationId === activeId) void check();
    }).then(u => { unlisten = u; }).catch(() => {});
    void listen('tasks-changed', () => void check()).then(u => { const prev = unlisten; unlisten = () => { prev?.(); u(); }; }).catch(() => {});
    return () => { alive = false; window.clearInterval(timer); unlisten?.(); };
  }, [activeId]);

  const [liveActivity, setLiveActivity] = useState<{ state?: string; activity?: string } | null>(null);

  const [chatError, setChatError] = useState('');
  // The conversation's isolated task workspace (git worktree or snapshot
  // copy). While one is active the backend resolves every tool/terminal/
  // execution path to it; Review tab applies or discards the result.
  const [taskWorkspace, setTaskWorkspace] = useState<TaskWorkspace | null>(null);
  useEffect(() => {
    if (!activeId || !nativeAvailable) { setTaskWorkspace(null); return; }
    let cancelled = false;
    api.taskWorkspaceStatus(activeId).then(task => { if (!cancelled) setTaskWorkspace(task); }).catch(() => {});
    return () => { cancelled = true; };
  }, [activeId]);
  const isolateTask = useCallback(async () => {
    if (!activeId || !nativeAvailable) return;
    try {
      setTaskWorkspace(await api.taskWorkspaceCreate(activeId));
    } catch (error) {
      setChatError(errorMessage(error));
    }
  }, [activeId]);

  const [chatNotice, setChatNotice] = useState('');

  const [modelBusy, setModelBusy] = useState(false);

  const [savingTools, setSavingTools] = useState(false);

  const [exporting, setExporting] = useState(false);

  const [exportedPath, setExportedPath] = useState('');

  const [preset, setPreset] = useState<string>('standard');

  const toolSave = useRef(false);

  const [error, setError] = useState('');

  const [search, setSearch] = useState('');
  const [searchOpen, setSearchOpen] = useState(false);

  const [collapsed, setCollapsed] = useState(false);

  // Below ~960px the sidebar leaves the grid and becomes a dismissible
  // overlay so the composer and primary actions stay reachable on the
  // minimum 760×560 window. It starts closed; the menubar toggle opens it.
  const [narrow, setNarrow] = useState(() => window.matchMedia?.('(max-width: 960px)').matches ?? false);
  const [sidebarOverlay, setSidebarOverlay] = useState(false);
  const sidebarToggle = useRef<HTMLButtonElement>(null);
  const sidebarOverlayEl = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const query = window.matchMedia?.('(max-width: 960px)');
    if (!query) return;
    const change = (event: MediaQueryListEvent) => setNarrow(event.matches);
    query.addEventListener('change', change);
    return () => query.removeEventListener('change', change);
  }, []);

  // Crossing the breakpoint either way closes the overlay; the wide layout
  // then shows the sidebar per its own collapsed state again.
  useEffect(() => { setSidebarOverlay(false); }, [narrow]);

  const closeSidebarOverlay = useCallback(() => {
    setSidebarOverlay(false);
    sidebarToggle.current?.focus();
  }, []);

  // Opening moves focus into the panel; Escape closes it and returns focus
  // to the menubar toggle.
  useEffect(() => {
    if (!narrow || !sidebarOverlay) return;
    const el = sidebarOverlayEl.current;
    el?.focus();
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !document.querySelector('dialog[open]')) {
        event.preventDefault();
        closeSidebarOverlay();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [narrow, sidebarOverlay, closeSidebarOverlay]);

  function openSearch() {
    // At narrow widths the search field lives in the overlay sidebar.
    if (narrow) setSidebarOverlay(true); else setCollapsed(false);
    setSearchOpen(true);
    setTimeout(() => document.querySelector<HTMLInputElement>('input[aria-label="Search conversations"]')?.focus(), 0);
  }

  const [renaming, setRenaming] = useState(false);

  const [title, setTitle] = useState('');

  const [appearance, setAppearance] = useState(loadAppearance);
  const [systemDark, setSystemDark] = useState(() => window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? true);
  useEffect(() => {
    const query = window.matchMedia?.('(prefers-color-scheme: dark)');
    if (!query) return;
    const change = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    query.addEventListener('change', change);
    return () => query.removeEventListener('change', change);
  }, []);
  const resolvedTheme = appearance.theme === 'system' ? (systemDark ? 'dark' : 'light') : appearance.theme;

  const selection = useRef(0);
  const settingsSave = useRef(0);

  // Detect a llama-server that exits while it should be loading or running.
  useRuntimeStatusPoll(data.runtime.phase, useCallback(runtime => setData(current => {
    const existing = current.runtime;
    if (existing.phase === runtime.phase && existing.message === runtime.message && existing.modelPath === runtime.modelPath) return current;
    return { ...current, runtime };
  }), [setData]));

  const active = data.conversations.find(item => item.id === activeId);

  const busy = generating || modelBusy || loading || savingTools || exporting;

  const modelSelection: ModelSelection = active ? { providerId: active.providerId, modelId: active.modelId ?? '' } : data.preferredModel;

  const selectedProvider = modelSelection.providerId ? data.providers.find(provider => provider.id === modelSelection.providerId) : undefined;

  const selectedRemoteModel = selectedProvider?.models.find(model => model.id === modelSelection.modelId);

  // Verified loopback engines (llama.cpp servers, FreeToken) run without an API key.

  const providerLoopback = Boolean(selectedProvider && localInferenceProvider(selectedProvider));

  const remoteReady = Boolean(providerLoopback && selectedProvider?.verified && selectedRemoteModel?.contextLength);

  const chatReady = modelSelection.providerId ? remoteReady && !active?.providerSelectionRequired : nativeAvailable && data.runtime.phase === 'ready';

  const modelLabel = modelSelection.providerId ? selectedProvider ? `${selectedProvider.name} · ${modelSelection.modelId || 'Choose a model'}` : 'Provider deleted · choose another' : data.runtime.phase === 'ready' ? `Local · ${data.runtime.modelPath?.split(/[\\/]/).pop() || 'llama.cpp'}` : 'Local model';

  function navigate(nextPage: Page) {

    setError('');

    setPage(nextPage);

    // Settings take over the whole window; a narrow-window overlay must not
    // pop back open when the user returns to chat.
    if (nextPage !== 'chat') setSidebarOverlay(false);

  }

  async function newConversation(targetProjectId: string | null = null) {

    setChatNotice('');

    const index = nativeAvailable && api.workspaceIndex
      ? await api.workspaceIndex().catch(() => workspaceIndex)
      : workspaceIndex;

    const resolvedProject = [targetProjectId, projectId]
      .map(id => (id ? findProject(index, id) : undefined))
      .find(Boolean)?.id ?? null;

    if (!resolvedProject) {
      setError('Add a project folder first, then start a chat.');
      return;
    }

    const resolvedFolder = findProject(index, resolvedProject);

    setWorkspaceIndex(index);

    switchDraft(null);

    selection.current++; setActiveId(null); setMessages([]); setPage('chat'); setError(''); setChatError(''); setLiveActivity(null); setRenaming(false);

    setAccessMode(rememberedTools.current.accessMode ?? 'ask');

    setInteractionMode(rememberedTools.current.interactionMode ?? 'act');

    setPlanReady(false);

    setPreset('standard');

    setSelectedConnectors(sourcesForNewConversation(rememberedTools.current.sources, Boolean(resolvedFolder?.path)));
    setSelectedTools(rememberedTools.current.tools);

    setProjectId(resolvedProject);

    try {
      if (nativeAvailable && api.setWorkspace) {
        await api.setWorkspace(resolvedFolder?.path ?? '');
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

        await api.saveConversationTools(conversationId, { sources, tools, accessMode: mode, interactionMode });

        savedConversation = true;

      }

      if (nativeAvailable) {

        const remembered = { sources, tools, accessMode: mode, interactionMode: rememberedTools.current.interactionMode };

        await api.saveRememberedTools(remembered);

        rememberedTools.current = remembered;

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

  // Ask/Plan/Act: saved on the conversation like the permission mode and
  // remembered as the default for new chats, without touching either tools
  // selection. Permitted tools and prompt injections follow on the next turn.
  async function changeInteractionMode(next: InteractionMode) {

    if (busy || toolSave.current || next === interactionMode) return;

    toolSave.current = true; setSavingTools(true); setError('');

    let savedConversation = false;

    let failure = '';

    try {

      if (activeId) {

        await api.saveConversationTools(activeId, { sources: selectedConnectors, tools: selectedTools, accessMode, interactionMode: next });

        savedConversation = true;

      }

      if (nativeAvailable) {

        const remembered = { ...rememberedTools.current, interactionMode: next };

        await api.saveRememberedTools(remembered);

        rememberedTools.current = remembered;

      }

    } catch (e) {

      failure = savedConversation

        ? `The mode is saved for this conversation, but remembering it for new chats failed: ${errorMessage(e)}`

        : `Your mode choice was not saved: ${errorMessage(e)}`;

    }

    if (savedConversation || !failure) setInteractionMode(next);

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

  useEffect(() => { applyAppearance(appearance, resolvedTheme); saveAppearance(appearance); }, [appearance, resolvedTheme]);

  useEffect(() => {

    if (!nativeAvailable) return;

    let disposed = false;

    api.bootstrap().then(value => {

      if (!disposed) {

        setData(normalizeBootstrap(value));

        // Startup restores the remembered selection for the initial new chat

        // while interaction is still blocked by `loading`.

        rememberedTools.current = value.rememberedTools ?? { sources: [], tools: [] };

        setSelectedConnectors(sourcesForNewConversation(rememberedTools.current.sources, false));

        setSelectedTools(rememberedTools.current.tools);
        setAccessMode(rememberedTools.current.accessMode ?? 'ask');
        setInteractionMode(rememberedTools.current.interactionMode ?? 'act');

      }

    }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setLoading(false); });

    return () => { disposed = true; };

  }, []);

  useEffect(() => {

    function keydown(event: KeyboardEvent) {

      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {

        event.preventDefault(); openSearch();

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

    setMessages([]); setSelectedConnectors([]); setSelectedTools([]); setAccessMode('ask'); setInteractionMode('act'); setPlanReady(false); setPreset('standard');

    try {

      const nextProject = conversationProjectId(workspaceIndex, id);

      // A running model operation holds the backend workspace lock and
      // rejects the switch; browsing a conversation must still work.
      if (nativeAvailable && api.setWorkspace) {
        try { await api.setWorkspace(findProject(workspaceIndex, nextProject)?.path ?? ''); }
        catch (e) { setError(errorMessage(e)); }
      }

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

        // Old conversations have no stored mode — Act keeps their behavior.

        setInteractionMode(tools.interactionMode ?? 'act');

        if (conversationPreset?.id) setPreset(conversationPreset.id);

      }

    }

    catch (e) { if (version === selection.current) { setError(errorMessage(e)); setActiveId(null); switchDraft(null); setSelectedConnectors(sourcesForNewConversation(rememberedTools.current.sources, false)); setSelectedTools(rememberedTools.current.tools); setAccessMode(rememberedTools.current.accessMode ?? 'ask'); } }

    finally { if (version === selection.current) setLoading(false); }

  }

  async function send(content: string, modeOverride?: InteractionMode) {

    if (busy || toolSave.current) throw new Error('Wait for the active operation before sending a message.');

    setChatNotice('');

    // "Implement plan" overrides only this turn's mode; the selector keeps
    // the user's choice so the next message follows it.

    const sendMode = modeOverride ?? interactionMode;

    setPlanReady(false);

    let id = activeId;

    // Chats live under a folder: a brand-new conversation needs one up front.
    const folderId = id ? projectId
      : (projectId && findProject(workspaceIndex, projectId) ? projectId : allProjects(workspaceIndex)[0]?.id) ?? null;

    if (!id && !folderId) {
      const issue = 'Add a project folder first, then send.';
      setChatError(issue);
      throw new Error(issue);
    }

    setGenerating(true); setChatError(''); setLiveActivity({ state: 'Preparing', activity: 'Preparing request…' });

    const previousIds = new Set(messages.map(message => message.id));

    try {

      if (!id) {

        const conversation = await api.createConversation(folderId); id = conversation.id;

        setProjectId(folderId);

        setWorkspaceIndex(await api.saveTaskMeta(id, { projectId: folderId, archived: false, pinned: false }));

        try { moveNewDraft(id); } catch (e) { setError(errorMessage(e)); }

        draftId.current = id;

        setActiveId(id); setData(current => ({ ...current, conversations: [conversation, ...current.conversations] }));

        // Persist the profile picked for this new chat before the turn is

        // assembled, so the backend renders the same payload the user chose.

        if (preset !== 'standard' && api.setPreset) {

          try { await api.setPreset(id, preset); } catch (e) { setError(errorMessage(e)); }

        }

      }

      // Persist the selector's choice, not a per-turn override — a Plan-mode
      // conversation stays Plan even while "Implement plan" runs an Act turn.
      await api.saveConversationTools(id, { sources: selectedConnectors, tools: selectedTools, accessMode, interactionMode });

      const conversationId = id;

      setMessages(current => [...current, { id: 'pending-user', conversationId, role: 'user', content, reasoning: '', status: 'complete', createdAt: Date.now() }]);

      await api.sendMessage(conversationId, content, event => {

        if (event.notice) { setChatNotice(event.notice); return; }

        if (event.context) setContextUsage({ conversationId, usage: event.context });

        const metrics = event.metrics;

        if (metrics) setMessages(current => current.map(message =>
          message.id === event.messageId ? { ...message, metrics } : message
        ));

        if ('approval' in event) setApproval(event.approval || null);

        if (event.planReady) setPlanReady(true);

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

        if (!event.messageId || (!event.content && !event.reasoning)) return;

        setMessages(current => {

          const existing = current.find(message => message.id === event.messageId);

          if (existing) return current.map(message => message.id === event.messageId ? { ...message, content: message.content + event.content, reasoning: message.reasoning + event.reasoning } : message);

          return [...current, { id: event.messageId, conversationId, role: 'assistant', content: event.content, reasoning: event.reasoning, status: 'streaming', createdAt: Date.now() }];

        });

      }, selectedConnectors, selectedTools, false, sendMode);

    } catch (e) {

      setChatError(errorMessage(e));

      // Restore the draft only when persistence confirms the message was not accepted.

      // An inference failure can happen after the user message has already been saved.

      if (!id) throw e;

      let saved: Message[];

      try { saved = await api.messages(id); }

      catch {

        setChatError(`${errorMessage(e)} Could not verify whether your message was saved. Reopen the conversation before sending it again.`);

        throw e;

      }

      setMessages(saved);

      if (!saved.some(message => message.role === 'user' && !previousIds.has(message.id) && message.content === content.trim())) throw e;

    }

    finally {

      try { if (id) setMessages(await api.messages(id)); setData(normalizeBootstrap(await api.bootstrap())); setWorkspaceIndex(await api.workspaceIndex()); }

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

      // Loading may apply a saved per-model profile. Refresh the persisted

      // preferences so the settings page shows the executable actually used

      // by the server.

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

    catch (e) {

      setError(errorMessage(e));

      // Refresh real status instead of manufacturing a null runtime — the
      // backend restores the previous model when a load fails, so the old
      // process may still be running.

      let refreshed = false;

      try { setData(normalizeBootstrap(await api.bootstrap())); refreshed = true; } catch { /* keep current data */ }

      // runtimeStatus runs last: it is the freshest view of the process.

      try { const runtime = await api.runtimeStatus(); setData(current => ({ ...current, runtime })); refreshed = true; } catch { /* keep current data */ }

      if (!refreshed) setError(`${errorMessage(e)} · Could not refresh model status`);

    }

    finally { setModelBusy(false); }

  }

  async function removeConversation() {

    if (!activeId || busy) return;

    if (!await confirm('This permanently deletes this conversation from your device.', { title: 'Delete conversation?', kind: 'warning' })) return;

    try { await api.deleteConversation(activeId); try { saveDraft(activeId, ''); } catch (e) { setError(errorMessage(e)); } setData(current => ({ ...current, conversations: current.conversations.filter(item => item.id !== activeId) })); newConversation(); }

    catch (e) { setError(errorMessage(e)); }

  }

  async function removeConversations(ids: string[]) {

    if (!ids.length) return;

    try {
      for (const id of ids) await api.deleteConversation(id);
      setData(current => ({ ...current, conversations: current.conversations.filter(item => !ids.includes(item.id)) }));
      if (activeId && ids.includes(activeId)) {
        selection.current++; setActiveId(null); setMessages([]); setRenaming(false);
        await newConversation();
      }
    }

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

  async function restoreConversation(id: string) {
    try { setWorkspaceIndex(await api.saveTaskMeta(id, { ...(workspaceIndex.tasks[id] ?? { projectId: null, pinned: false }), archived: false })); }
    catch (e) { setError(errorMessage(e)); }
  }

  const activeProject = allProjects(workspaceIndex).find(p => p.id === projectId);
  const activeProjectName = activeProject?.name ?? '';

  const banners = <>
    {!nativeAvailable && <div className="preview-banner">Browser preview · Open the desktop application to load models and save conversations.</div>}
    {exportedPath && <div className="preview-banner" role="status">Saved conversation to {exportedPath}<button className="icon-button" aria-label="Dismiss export confirmation" onClick={() => setExportedPath('')}><X size={16} /></button></div>}
    {error && <div className="error-banner" role="alert"><span>{error}</span><button className="icon-button" aria-label="Dismiss error" onClick={() => setError('')}><X size={16} /></button></div>}
  </>;

  return <div className={`app ${collapsed || narrow || page !== 'chat' ? 'sidebar-collapsed' : ''}`}>

    <div className="app-menubar" aria-label="Application toolbar">

      <div className="menubar-nav-group">
        <button
          ref={sidebarToggle}
          className="icon-button menubar-nav-btn menubar-sidebar-toggle"
          aria-label="Toggle sidebar"
          aria-expanded={narrow ? sidebarOverlay : !collapsed}
          aria-controls="app-sidebar"
          title="Toggle sidebar"
          onClick={() => narrow ? setSidebarOverlay(open => !open) : setCollapsed(!collapsed)}
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

      <details className="app-menu" name="application-menu"><summary>Edit</summary><div><button disabled={!active || busy} onClick={event => { if (active) { setTitle(active.title); setRenaming(true); } event.currentTarget.closest('details')?.removeAttribute('open'); }}>Rename task</button><button onClick={event => { event.currentTarget.closest('details')?.removeAttribute('open'); openSearch(); }}>Search tasks <kbd>Ctrl+K</kbd></button></div></details>

      <details className="app-menu" name="application-menu"><summary>View</summary><div><button onClick={() => setCollapsed(!collapsed)}>Toggle sidebar</button><button onClick={() => setAppearance(current => ({ ...current, theme: resolvedTheme === 'dark' ? 'light' : 'dark' }))}>Switch to {resolvedTheme === 'dark' ? 'light' : 'dark'} theme</button><button onClick={event => { navigate('general'); event.currentTarget.closest('details')?.removeAttribute('open'); }}>Open settings</button></div></details>

      <details className="app-menu" name="application-menu"><summary>Help</summary><div className="shortcut-help"><strong>Workspace shortcuts</strong><span>Review <kbd>Ctrl+Shift+G</kbd></span><span>Terminal <kbd>Ctrl+`</kbd></span><span>Browser <kbd>Ctrl+T</kbd></span><span>Files <kbd>Ctrl+P</kbd></span></div></details>

      <div className="titlebar-drag-region" data-tauri-drag-region aria-hidden="true" />

      <WindowControls onError={setError} />
    </div>

    {page === 'chat' && (narrow
      ? sidebarOverlay && <>
          {/* Scrim click closes the overlay and returns focus to the toggle. */}
          <div className="sidebar-scrim" aria-hidden="true" onClick={closeSidebarOverlay} />
          <div
            id="app-sidebar"
            ref={sidebarOverlayEl}
            className="sidebar-overlay"
            role="dialog"
            aria-modal="true"
            aria-label="Conversations"
            tabIndex={-1}
          >
            <Sidebar workspace={workspaceIndex} onWorkspace={setWorkspaceIndex} projectId={projectId} onProject={id => void selectProject(id)} onDeleteConversations={ids => void removeConversations(ids)} onHit={(id, messageId) => { setHitId(messageId); void selectConversation(id); }} page={page} onPage={target => { setSidebarOverlay(false); navigate(target); }} conversations={data.conversations} activeId={activeId} onSelect={id => { setSidebarOverlay(false); void selectConversation(id); }} onNew={targetProjectId => { setSidebarOverlay(false); void newConversation(targetProjectId); }} busy={busy} search={search} onSearch={setSearch} searchOpen={searchOpen} onSearchOpenChange={setSearchOpen} theme={resolvedTheme} onTheme={() => setAppearance(current => ({ ...current, theme: resolvedTheme === 'dark' ? 'light' : 'dark' }))} onCollapse={closeSidebarOverlay} />
          </div>
        </>
      : !collapsed && <Sidebar id="app-sidebar" workspace={workspaceIndex} onWorkspace={setWorkspaceIndex} projectId={projectId} onProject={id => void selectProject(id)} onDeleteConversations={ids => void removeConversations(ids)} onHit={(id, messageId) => { setHitId(messageId); void selectConversation(id); }} page={page} onPage={navigate} conversations={data.conversations} activeId={activeId} onSelect={id => void selectConversation(id)} onNew={targetProjectId => void newConversation(targetProjectId)} busy={busy} search={search} onSearch={setSearch} searchOpen={searchOpen} onSearchOpenChange={setSearchOpen} theme={resolvedTheme} onTheme={() => setAppearance(current => ({ ...current, theme: resolvedTheme === 'dark' ? 'light' : 'dark' }))} onCollapse={() => setCollapsed(true)} />)}

    {approval && <ApprovalDialog key={approval.id} request={approval} onResolve={allow => api.resolveToolApproval(approval.id, allow)} onResolveAskUser={(choice) => api.resolveAskUser(approval.id, choice)} />}

    {page === 'chat' ? <main className="workspace">
      <header className={`workspace-header ${!messages.length && !renaming ? 'workspace-header-minimal' : ''}`}>
        <div className="workspace-lead">
          {collapsed && !narrow && <button className="icon-button" aria-label="Expand sidebar" onClick={() => setCollapsed(false)}><PanelLeftOpen size={17} /></button>}
          {renaming
            ? <form className="rename-form" onSubmit={event => { event.preventDefault(); void rename(); }}><input autoFocus aria-label="Conversation title" maxLength={160} value={title} onChange={e => setTitle(e.target.value)} /><button className="icon-button" aria-label="Save title"><Check size={16} /></button><button type="button" className="icon-button" aria-label="Cancel rename" onClick={() => setRenaming(false)}><X size={16} /></button></form>
            : <span className="workspace-title">{active?.title || 'New conversation'}</span>}
          {activeProjectName && !renaming && <span className="workspace-project"><Folder size={13} aria-hidden="true" />{activeProjectName}</span>}
        </div>
        <div className="header-actions">
          {allProjects(workspaceIndex).length > 0 && <select aria-label="Task project" value={projectId ?? allProjects(workspaceIndex)[0]?.id} disabled={busy} onChange={e => void selectProject(e.target.value)}>{allProjects(workspaceIndex).map(p => <option value={p.id} key={p.id}>{p.name}</option>)}</select>}
          {active && <details className="app-menu task-menu" name="application-menu"><summary aria-label="Task actions"><MoreHorizontal size={17} /></summary><div><button className="icon-button" title="Rename conversation" aria-label="Rename conversation" disabled={busy} onClick={() => { setTitle(active.title); setRenaming(true); }}><Pencil size={15} /></button><button className="icon-button" title="Export conversation" aria-label="Export conversation" disabled={!messages.length || busy} onClick={() => void exportChat()}><Download size={15} /></button><button className="icon-button" title="Delete conversation" aria-label="Delete conversation" disabled={busy} onClick={() => void removeConversation()}><Trash2 size={15} /></button></div></details>}
          <WorkspacePanel busy={busy} revision={`${projectId}:${activeId}`} conversation={activeId} taskTick={messages.length} onTaskStatus={setTaskWorkspace} />
        </div>
      </header>
      <DownloadActivity />
      {banners}
      <Chat
        projectName={activeProjectName}
        attachmentDrafts={attachmentDrafts}
        onAttachmentDraftsChange={setAttachmentDrafts}
        conversationKey={activeId ?? 'new'}
        composerTools={<PermissionSelector accessMode={accessMode} onAccessModeChange={mode => void changeTools(selectedConnectors, selectedTools, mode)} busy={busy} nativeAvailable={nativeAvailable} />}
        contextUsage={contextUsage?.conversationId === activeId ? contextUsage.usage : undefined}
        modelLabel={modelLabel}
        modelKey={JSON.stringify([modelSelection, data.runtime.modelPath, data.runtime.loadedConfig, selectedRemoteModel, data.preferences])}
        selectionIssue={active?.providerSelectionRequired || (Boolean(modelSelection.providerId) && !remoteReady)}
        draft={draft}
        onDraftChange={changeDraft}
        messages={messages}
        generating={generating || (runningTask != null && runningTask.status !== 'paused')}
        ready={chatReady}
        loading={loading}
        disabled={savingTools || exporting}
        preset={preset}
        connectorIds={selectedConnectors}
        connectorTools={selectedTools}
        onSend={send}
        onQueue={activeId ? async (content) => {
          await api.taskEnqueue(activeId, content, interactionMode);
          setMessages(current => [...current, { id: `queued-${Date.now()}`, conversationId: activeId, role: 'user', content, reasoning: '', status: 'complete', createdAt: Date.now() }]);
        } : undefined}
        onSteer={runningTask ? async (content) => api.taskSteer(runningTask.id, content) : undefined}
        onCancel={() => {
          if (!generating && runningTask) {
            void api.taskCancel(runningTask.id).catch(e => setChatError(errorMessage(e)));
          } else {
            void api.cancelGeneration().catch(e => setChatError(errorMessage(e)));
          }
        }}
        onConfigure={() => navigate('models')}
        interactionMode={interactionMode}
        onInteractionModeChange={mode => void changeInteractionMode(mode)}
        taskWorkspace={taskWorkspace}
        onIsolateTask={activeId && !taskWorkspace ? () => void isolateTask() : undefined}
        planReady={planReady}
        onImplementPlan={() => {
          const prompt = 'Implement the plan in your previous message. Work through the checklist one task at a time, updating the todo tools as steps complete. If a step cannot be completed, say so honestly instead of skipping it.';
          void send(prompt, 'act').catch(e => setChatError(errorMessage(e)));
        }}
        onConfigureLocalModel={filename => { setRequestedModel(filename); navigate('models'); }}
        liveActivity={liveActivity}
        providers={data.providers}
        onSelectModel={saveModelSelection}
        chatError={chatError}
        chatNotice={chatNotice}
        onDismissError={() => setChatError('')}
      />
      <Trajectory
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
      />
    </main> : <main className="workspace settings-mode">
      <DownloadActivity />
      {banners}
      <SettingsShell section={page} onSection={navigate} onClose={() => navigate('chat')}>
        <Suspense fallback={<p role="status" className="loading-state">Opening settings…</p>}>
        {page === 'general' ? <GeneralSettings accessMode={accessMode} onAccessModeChange={mode => void changeTools(selectedConnectors, selectedTools, mode)} busy={busy} nativeAvailable={nativeAvailable} />
          : page === 'appearance' ? <AppearanceSettings appearance={appearance} onChange={setAppearance} />
          : page === 'shortcuts' ? <ShortcutSettings />
          : page === 'data' ? <DataSettings workspace={workspaceIndex} conversations={data.conversations} activeTitle={active?.title} canExport={Boolean(active && messages.length)} onExport={() => void exportChat()} onRestore={id => void restoreConversation(id)} busy={busy} />
          : page === 'tools' ? <ToolsSettings accessMode={accessMode} interactionMode={interactionMode} onInteractionModeChange={mode => void changeInteractionMode(mode)} preset={preset} onPresetChange={changePreset} selectedTools={selectedTools} onToolsChange={tools => void changeTools(selectedConnectors, tools)} selected={selectedConnectors} onChange={sources => void changeTools(sources, selectedTools)} scopeLabel={active?.title} conversationId={activeId} busy={busy} loading={loading} saving={savingTools} onNavigate={target => navigate(target)} />
          : page === 'connectors' ? <Connectors />
          : page === 'skills' ? <Skills conversationId={activeId} />
          : page === 'execution' || page === 'web-search' ? <Execution section={page} />
          : page === 'browser' ? <BrowserSettings />
          : page === 'documents' ? <Documents />
          : <Models section={page} onSection={navigate} onRefresh={async () => { setData(normalizeBootstrap(await api.bootstrap())); }} requestedModel={requestedModel} onRequestedModelHandled={() => setRequestedModel(undefined)} config={data.config} preferences={data.preferences} runtime={data.runtime} busy={!nativeAvailable || busy} onLoad={() => void modelAction(true)} onUnload={() => void modelAction(false)} onSaveSelection={saveModelSelection} onSaveConfig={async config => {
            const request = ++settingsSave.current;
            try { await api.saveConfig(config); } catch (e) { setError(errorMessage(e)); throw e; }
            try {
              const next = normalizeBootstrap(await api.bootstrap());
              if (request === settingsSave.current) setData(next);
            } catch (refreshError) {
              return `Saved, but settings could not be refreshed: ${errorMessage(refreshError)}`;
            }
          }} onSavePreferences={async preferences => {
            const request = ++settingsSave.current;
            try { await api.savePreferences(preferences); } catch (e) { setError(errorMessage(e)); throw e; }
            try {
              const next = normalizeBootstrap(await api.bootstrap());
              if (request === settingsSave.current) setData(next);
            } catch (refreshError) {
              return `Saved, but settings could not be refreshed: ${errorMessage(refreshError)}`;
            }
          }} />}
        </Suspense>
      </SettingsShell>
    </main>}

  </div>;

}
