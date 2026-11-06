import { toolOutput } from './lib/toolOutput';
import { publishTurnItem } from './lib/turnItems';
import { CommandPalette } from './components/CommandPalette';
import { useEffect, useRef, useState, type SetStateAction } from 'react';
import { useSessionRuns } from './hooks/useSessionRuns';
import { Check, ChevronDown, Download, Pencil, Trash2, X, ArrowLeft, ArrowRight, Folder, MoreHorizontal } from 'lucide-react';

import { confirm, save } from '@tauri-apps/plugin-dialog';

import { api, errorMessage, nativeAvailable } from './lib/api';

import { defaultRuntimeConfig, type Bootstrap, type Message, type PendingTurn, type ToolApproval, type ToolSelection, type AccessMode, type ModelSelection, type RememberedTools } from './lib/types';

import { Sidebar, type Page } from './components/Sidebar';

import type { Attachment } from './lib/attachments';

import { Chat } from './components/Chat';

import { Models } from './components/Models';

import { DownloadActivity } from './components/DownloadActivity';

import { Skills } from './components/Skills';
import { PermissionRulesSettings } from './components/PermissionRulesSettings';
import { PluginSettings } from './components/PluginSettings';

import { SecuritySettings } from './components/SecuritySettings';
import { GitSettings } from './components/GitSettings';
import { WeatherSettings } from './components/WeatherSettings';
import { Execution } from './components/Execution';
import { DevelopmentSettings } from './components/DevelopmentSettings';
import { LanguageServerSettings } from './components/LanguageServerSettings';
import { NativeExecutionSettings } from './components/NativeExecutionSettings';
import { AutomationSettings } from './components/AutomationSettings';
import { HooksSettings } from './components/HooksSettings';

import { Connectors } from './components/Connectors';

import { ApprovalDialog, ToolsSettings, PermissionSelector } from './components/ToolControls';

import { Trajectory } from './components/Trajectory';

import { WindowControls } from './components/WindowControls';
import { WorkspacePanel } from './components/WorkspacePanel';
import { GeneralSettings } from './components/GeneralSettings';
import { AppearanceSettings } from './components/AppearanceSettings';
import { TabDock } from './components/TabDock';
import { SettingsModal } from './components/SettingsModal';
import { useAppearance } from './lib/useAppearance';

import type { WorkspaceIndex } from './lib/types';

import { allProjects, conversationProjectId, findProject } from './lib/projects';
import { remoteModelReady } from './lib/providers';

import './styles.css';

import './workspace.css';

import './models.css';
import './desktop.css';
import './appearance.css';
import './responses.css';
import './sessions.css';

import { loadDraft, saveDraft, moveNewDraft } from './lib/drafts';



const initialData: Bootstrap = {

  conversations: [], config: defaultRuntimeConfig,

  preferences: { runtimePath: '', modelPath: '', projectorPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, generativeUi: false, systemPrompt: 'You are a helpful local assistant. Be clear and accurate. If you do not know something, say so.' },

  runtime: { phase: 'stopped', message: 'No model loaded', modelPath: null },

  rememberedTools: { sources: [], tools: [] },

  providers: [], preferredModel: { providerId: null, modelId: '' },

};

function normalizeBootstrap(value: Bootstrap): Bootstrap {

  return { ...value, providers: value.providers ?? [], preferredModel: value.preferredModel ?? { providerId: null, modelId: '' }, rememberedTools: value.rememberedTools ?? { sources: [], tools: [] } };

}

function sourcesForNewConversation(sources: string[], hasWorkspace: boolean) {

  return hasWorkspace ? [...new Set([...sources, '__workspace', '__execution'])] : sources.filter(source => source !== '__workspace' && source !== '__execution');

}



export default function App() {

  const [workspacePanelHost, setWorkspacePanelHost] = useState<HTMLDivElement | null>(null);
  const [workspaceIndex, setWorkspaceIndex] = useState<WorkspaceIndex>({ projects: [], tasks: {} });

  const [projectId, setProjectId] = useState<string | null>(null);
  const projectIdRef = useRef(projectId);
  projectIdRef.current = projectId;

  const [navigation, setNavigation] = useState<{ ids: string[]; cursor: number }>({ ids: [], cursor: -1 });

  const [hitId, setHitId] = useState<string | null>(null);

  useEffect(() => { if (nativeAvailable && api.workspaceIndex) api.workspaceIndex().then(setWorkspaceIndex).catch(e => setError(errorMessage(e))); }, []);

  async function selectProject(id: string | null) {

    const target = id || null;
    const version = selection.current;
    const conversationId = activeId;

    try {

      const project = findProject(workspaceIndex, target);

      await syncNavigationWorkspace(project?.path ?? '', version);
      if (version !== selection.current) return;

      if (conversationId) setWorkspaceIndex(await api.saveTaskMeta(conversationId, { ...(workspaceIndex.tasks[conversationId] ?? { archived: false, pinned: false }), projectId: target }));
      if (version !== selection.current) return;

      setProjectId(target);

    } catch (e) { if (version === selection.current) setError(errorMessage(e)); }

  }

  async function chooseTaskWorkspace(path: string): Promise<string> {
    const result = await api.setTaskWorkspace(path, activeId);
    setWorkspaceIndex(result.workspace);
    setProjectId(result.project.id);
    return result.project.path;
  }

  const [attachmentDrafts, setAttachmentDrafts] = useState<Record<string, Attachment[]>>({});

  const draftId = useRef<string | null>(null);

  const draftValue = useRef(loadDraft(null));
  const draftRevision = useRef(0);

  const [draft, setDraft] = useState(draftValue.current);

  function changeDraft(value: string | ((previous: string) => string)) {

    const next = typeof value === 'function' ? value(draftValue.current) : value;
    draftRevision.current++;

    draftValue.current = next; setDraft(next);

    try { saveDraft(draftId.current, next); } catch (e) { setError(errorMessage(e)); }

  }

  function switchDraft(id: string | null) {

    draftRevision.current++;
    draftId.current = id; draftValue.current = loadDraft(id); setDraft(draftValue.current);

  }

  const [selectedConnectors, setSelectedConnectors] = useState<string[]>([]);

  const [accessMode, setAccessMode] = useState<AccessMode>('ask');

  // Planning is scoped to the conversation. Its read-only investigation
  // precedes plan approval and the normal tool loop.



  const [selectedTools, setSelectedTools] = useState<ToolSelection[]>([]);

  // Last explicitly chosen selection for future chats; restored at startup and

  // applied by newConversation. Existing conversations never read from it.

  const rememberedTools = useRef<RememberedTools>({ sources: [], tools: [] });



  const [data, setData] = useState(initialData);

  const [page, setPage] = useState<Page>('chat');

  const [requestedModel, setRequestedModel] = useState<string>();

  const [activeId, setActiveId] = useState<string | null>(null);

  const [messages, setMessages] = useState<Message[]>([]);
  const displayMessages = setMessages;
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;
  const sessions = useSessionRuns();
  const session = sessions.get(activeId ?? 'new');
  const { generating, activity: liveActivity, error: chatError, notice: chatNotice, planMode } = session;
  const setChatError = (error: string) => sessions.update(activeId ?? 'new', { error });
  const setChatNotice = (notice: string) => sessions.update(activeId ?? 'new', { notice });
  const setPlanMode = (planMode: boolean) => sessions.update(activeId ?? 'new', { planMode });
  const contextUsage = session.context ? { conversationId: activeId, usage: session.context } : null;
  const approvalOrder = useRef<string[]>([]);
  const approvalEntries = Object.entries(sessions.runs).flatMap(([id, run]) => run.approvals.map(request => ({ id, request })));
  const approvalEntry = approvalEntries.find(entry => entry.request.id === approvalOrder.current[0]) ?? approvalEntries[0];
  const approval = approvalEntry?.request;
  const [reviewRequest, setReviewRequest] = useState(0);


  const [loading, setLoading] = useState(nativeAvailable);

  useEffect(() => {

    if (loading || !hitId) return;

    const element = document.querySelector(`[data-message-id="${CSS.escape(hitId)}"]`);

    if (element) { element.scrollIntoView({ block: 'center' }); element.classList.add('search-highlight'); setHitId(null); }

  }, [messages, loading, hitId]);

  const [pausedConversationId, setPausedConversationId] = useState<string | null>(null);
  const [savedPending, setSavedPending] = useState<PendingTurn | null>(null);
  const pendingSends = useRef(new Map<string, { cancelled: boolean; backendStarted: boolean }>());
  const recoveredRuns = useRef(new Map<string, string>());
  const [checkingSessions, setCheckingSessions] = useState(nativeAvailable && typeof api.listActiveRuns === 'function');

  // A reloaded webview no longer owns the original IPC channel. Discover
  // backend-owned turns and refresh their persisted text and pending prompts;
  // a local sender remains the sole owner of its own stream callbacks.
  useEffect(() => {
    if (!nativeAvailable || !api.listActiveRuns) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const recoveryNotice = 'Reconnected to a running response. Saved output updates shortly.';
    const bounded = async <T,>(request: Promise<T>): Promise<T> => {
      let timeout: ReturnType<typeof setTimeout> | undefined;
      try {
        return await Promise.race([request, new Promise<never>((_, reject) => {
          timeout = setTimeout(() => reject(new Error('The request timed out.')), 5000);
        })]);
      } finally { if (timeout) clearTimeout(timeout); }
    };
    const poll = async () => {
      try {
        const snapshots = await bounded(api.listActiveRuns());
        if (disposed) return;
        setError(current => current.startsWith('Could not check running responses:') ? '' : current);
        const active = new Map(snapshots.map(snapshot => [snapshot.conversationId, snapshot]));
        await Promise.all(snapshots.map(async snapshot => {
          const id = snapshot.conversationId;
          if (pendingSends.current.has(id)) return;
          recoveredRuns.current.set(id, snapshot.instanceId);
          sessions.update(id, { generating: true, approvals: snapshot.approvals,
            activity: { state: snapshot.phase === 'queued' ? 'queued' : snapshot.state ?? undefined,
              activity: snapshot.cancellationRequested ? 'Stopping response…' : snapshot.activity ?? (snapshot.phase === 'queued' ? 'Waiting for a response slot…' : 'Response running…') },
            notice: recoveryNotice });
          try {
            const saved = await bounded(api.messages(id));
            if (disposed || pendingSends.current.has(id) || recoveredRuns.current.get(id) !== snapshot.instanceId) return;
            sessions.update(id, { messages: saved, error: '' });
            if (activeIdRef.current === id) displayMessages(saved);
          } catch (e) {
            if (!disposed && recoveredRuns.current.get(id) === snapshot.instanceId) sessions.update(id, { error: `Could not refresh this response: ${errorMessage(e)}` });
          }
        }));
        if (!disposed) setCheckingSessions(false);
        await Promise.all([...recoveredRuns.current].map(async ([id, instance]) => {
          if (active.has(id) || pendingSends.current.has(id)) return;
          try {
            const saved = await bounded(api.messages(id));
            if (disposed || pendingSends.current.has(id) || recoveredRuns.current.get(id) !== instance) return;
            recoveredRuns.current.delete(id);
            sessions.update(id, run => ({ ...run, messages: saved, generating: false, approvals: [], activity: null,
              notice: run.notice === recoveryNotice ? '' : run.notice, error: '', unread: activeIdRef.current !== id }));
            if (activeIdRef.current === id) displayMessages(saved);
            if (api.getPendingTurn) {
              const pending = await bounded(api.getPendingTurn(id));
              if (!disposed && !pendingSends.current.has(id)) setData(current => ({ ...current, pendingConversationIds: pending
                ? [...new Set([...(current.pendingConversationIds ?? []), id])]
                : (current.pendingConversationIds ?? []).filter(value => value !== id) }));
            }
          } catch (e) {
            if (!disposed) sessions.update(id, { error: `Could not refresh this response: ${errorMessage(e)}` });
          }
        }));
      } catch (e) {
        if (!disposed) {
          setError(`Could not check running responses: ${errorMessage(e)}`);
          for (const id of recoveredRuns.current.keys()) sessions.update(id, { error: `Could not reconnect to this response: ${errorMessage(e)}` });
        }
      } finally {
        if (!disposed) timer = setTimeout(() => void poll(), recoveredRuns.current.size ? 750 : 5000);
      }
    };
    void poll();
    return () => { disposed = true; if (timer) clearTimeout(timer); };
  }, []);

  useEffect(() => {
    setPausedConversationId(null);
    if (!activeId || generating || !api.getConversationRun) return;
    let disposed = false;
    void api.getConversationRun(activeId).then(run => {
      if (!disposed && run?.status === 'paused') setPausedConversationId(activeId);
    }).catch(() => {});
    return () => { disposed = true; };
  }, [activeId, generating]);

  useEffect(() => {
    setSavedPending(null);
    if (!activeId || generating || !api.getPendingTurn) return;
    let disposed = false;
    void api.getPendingTurn(activeId).then(value => {
      if (disposed) return;
      setSavedPending(value);
      setData(current => ({ ...current, pendingConversationIds: value
        ? [...new Set([...(current.pendingConversationIds ?? []), activeId])]
        : (current.pendingConversationIds ?? []).filter(id => id !== activeId) }));
    })
      .catch(e => { if (!disposed) setChatError(`Could not check saved requests: ${errorMessage(e)}`); });
    return () => { disposed = true; };
  }, [activeId, generating]);







  const [modelBusy, setModelBusy] = useState(false);

  const [savingTools, setSavingTools] = useState(false);

  const [exporting, setExporting] = useState(false);

  const [exportedPath, setExportedPath] = useState('');

  const [preset, setPreset] = useState<string>('standard');

  const toolSave = useRef(false);

  const [error, setError] = useState('');

  const reasoningOverride: 'auto' | 'quick' | 'think' | undefined = undefined;
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [search, setSearch] = useState('');

  const [collapsed, setCollapsed] = useState(() => window.innerWidth <= 760);

  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return;
    const compactWindow = window.matchMedia('(max-width: 760px)');
    const collapseForCompactWindow = (event: MediaQueryListEvent) => {
      if (event.matches) setCollapsed(true);
    };
    compactWindow.addEventListener('change', collapseForCompactWindow);
    return () => compactWindow.removeEventListener('change', collapseForCompactWindow);
  }, []);

  const [renaming, setRenaming] = useState(false);

  const [title, setTitle] = useState('');

  const appearance = useAppearance();
  const { theme } = appearance;
  const toggleTheme = () => appearance.update({ theme: theme === 'dark' ? 'light' : 'dark' });

  const selection = useRef(0);
  // Completion refreshes can overlap each other and conversation creation.
  // Only the latest still-current snapshot may replace navigation metadata.
  const completionRefresh = useRef(0);
  const workspaceWrites = useRef<Promise<void>>(Promise.resolve());
  async function syncNavigationWorkspace(path: string, version: number) {
    const write = workspaceWrites.current.catch(() => {}).then(async () => {
      if (version === selection.current && nativeAvailable && api.setWorkspace) await api.setWorkspace(path);
    });
    workspaceWrites.current = write;
    await write;
  }

  const active = data.conversations.find(item => item.id === activeId);

  const busy = generating || modelBusy || loading || checkingSessions || savingTools || exporting;
  const anyGenerating = Object.values(sessions.runs).some(run => run.generating);
  const runningConversations = Object.entries(sessions.runs).filter(([, run]) => run.generating)
    .map(([id]) => data.conversations.find(item => item.id === id)?.title ?? 'New conversation');
  const navigationBusy = modelBusy || savingTools || exporting;
  const globalBusy = anyGenerating || modelBusy || loading || checkingSessions || savingTools || exporting;

  const modelSelection: ModelSelection = active ? { providerId: active.providerId, modelId: active.modelId ?? '' } : data.preferredModel;

  const selectedProvider = modelSelection.providerId ? data.providers.find(provider => provider.id === modelSelection.providerId) : undefined;

  const selectedRemoteModel = selectedProvider?.models.find(model => model.id === modelSelection.modelId);

  // Exact loopback detection — substring checks would trust URLs that merely
  // mention a loopback host in a path or userinfo segment.
  const remoteReady = Boolean(selectedProvider && selectedRemoteModel && remoteModelReady(selectedProvider, selectedRemoteModel));

  const chatReady = modelSelection.providerId ? remoteReady && !active?.providerSelectionRequired : nativeAvailable && data.runtime.phase === 'ready';

  const modelLabel = modelSelection.providerId ? selectedProvider ? `${selectedProvider.name} · ${modelSelection.modelId || 'Choose a model'}` : 'Provider deleted · choose another' : data.runtime.phase === 'ready' ? `Local · ${data.runtime.modelPath?.split(/[\\/]/).pop() || 'llama.cpp'}` : 'Local model';

  function navigate(nextPage: Page) {

    setError('');

    setPage(nextPage);

    // The sidebar is an overlay on narrow windows — fold it away on navigation.
    if (window.innerWidth <= 760) setCollapsed(true);

  }

  function searchConversations() {
    setPage('chat');
    setCollapsed(false);
    setTimeout(() => window.dispatchEvent(new Event('locallm-search')), 0);
  }

  function goHome() {
    switchDraft(null);
    const version = ++selection.current;
    setLoading(false);
    void syncNavigationWorkspace(activeProject?.path ?? '', version).catch(e => { if (version === selection.current) setError(errorMessage(e)); });
    activeIdRef.current = null; setActiveId(null);
    setMessages([]);
    navigate('chat');

    setRenaming(false);
    setAccessMode(rememberedTools.current.accessMode ?? 'ask');

    setPreset('standard');
    setSelectedConnectors(sourcesForNewConversation(rememberedTools.current.sources, Boolean(activeProject?.path)));
    setSelectedTools(rememberedTools.current.tools);
  }

  async function newConversation(targetProjectId: string | null = null) {

    const version = ++selection.current;
    setLoading(false);
    setChatNotice('');

    const index = nativeAvailable && api.workspaceIndex
      ? await api.workspaceIndex().catch(() => workspaceIndex)
      : workspaceIndex;

    if (version !== selection.current) return;

    const resolvedProject = [targetProjectId, projectId]
      .map(id => (id ? findProject(index, id) : undefined))
      .find(Boolean)?.id ?? allProjects(index)[0]?.id ?? null;

    if (!resolvedProject) {
      setError('Add a project folder first, then start a chat.');
      return;
    }

    const resolvedFolder = findProject(index, resolvedProject);

    setWorkspaceIndex(index);

    switchDraft(null);

    activeIdRef.current = null; setActiveId(null); setMessages([]); navigate('chat'); setError(''); setRenaming(false);

    setAccessMode(rememberedTools.current.accessMode ?? 'ask');



    setPreset('standard');

    setSelectedConnectors(sourcesForNewConversation(rememberedTools.current.sources, Boolean(resolvedFolder?.path)));
    setSelectedTools(rememberedTools.current.tools);

    setProjectId(resolvedProject);

    try {
      await syncNavigationWorkspace(resolvedFolder?.path ?? '', version);
    } catch (e) {
      if (version === selection.current) setError(errorMessage(e));
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

    } catch (e) { setError(errorMessage(e)); throw e; }

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

      }

    }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setLoading(false); });

    return () => { disposed = true; };

  }, []);

  useEffect(() => {

    function keydown(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === 'p') {
        event.preventDefault(); setPaletteOpen(true); return;
      }

      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {

        event.preventDefault(); searchConversations();

      }

      if ((event.ctrlKey || event.metaKey) && event.key === ',') {
        event.preventDefault(); navigate('general');
      }

      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'n') {

        event.preventDefault(); if (!navigationBusy) newConversation();

      }

    }

    window.addEventListener('keydown', keydown); return () => window.removeEventListener('keydown', keydown);

  }, [busy, navigationBusy, page, collapsed, projectId, workspaceIndex]);

  async function selectConversation(id: string, fromHistory = false) {

    if (navigationBusy || toolSave.current) return;

    setChatNotice('');

    const version = ++selection.current;

    switchDraft(id);

    navigate('chat'); activeIdRef.current = id; setActiveId(id); sessions.update(id, { unread: false }); setLoading(true); setError(''); setRenaming(false);

    setMessages([]); setSelectedConnectors([]); setSelectedTools([]); setAccessMode('ask'); setPreset('standard');

    try {

      const nextProject = conversationProjectId(workspaceIndex, id);

      await syncNavigationWorkspace(findProject(workspaceIndex, nextProject)?.path ?? '', version);
      if (version !== selection.current) return;

      setProjectId(nextProject);

      const [next, tools, conversationPreset] = await Promise.all([

        api.messages(id),

        api.conversationTools(id),

        api.getPreset ? api.getPreset(id).catch(() => null) : Promise.resolve(null),

      ]);

      if (version === selection.current) {

        if (!fromHistory) setNavigation(n => ({ ids: [...n.ids.slice(0, n.cursor + 1), id], cursor: n.cursor + 1 }));

        setMessages(sessions.get(id).generating ? sessions.get(id).messages : next);

        setSelectedConnectors(tools.sources);

        setSelectedTools(tools.tools);

        setAccessMode(tools.accessMode || 'ask');

        if (conversationPreset?.id) setPreset(conversationPreset.id);

      }

    }

    catch (e) { if (version === selection.current) { setError(errorMessage(e)); activeIdRef.current = null; setActiveId(null); switchDraft(null); setSelectedConnectors(sourcesForNewConversation(rememberedTools.current.sources, false)); setSelectedTools(rememberedTools.current.tools); setAccessMode(rememberedTools.current.accessMode ?? 'ask'); } }

    finally { if (version === selection.current) setLoading(false); }

  }

  async function send(content: string, recoveryId?: string, resumeFailedMessageId?: string) {

    if (busy || toolSave.current || pendingSends.current.has(activeId ?? 'new')) throw new Error('Wait for the active operation before sending a message.');

    let id = activeId;
    const creationSelection = selection.current;
    const updateRun = (patch: Parameters<typeof sessions.update>[1]) => sessions.update(id ?? 'new', patch);
    const setChatError = (error: string) => updateRun({ error });
    const setChatNotice = (notice: string) => updateRun({ notice });
    const setGenerating = (generating: boolean) => updateRun({ generating });
    const setLiveActivity = (activity: typeof liveActivity) => updateRun({ activity });
    const setApproval = (change: ToolApproval | null | ((current: ToolApproval | null) => ToolApproval | null)) => {
      const previous = sessions.get(id ?? 'new').approvals;
      const approvals = typeof change === 'function'
        ? previous.flatMap(item => { const next = change(item); return next ? [next] : []; })
        : change ? [...previous.filter(item => item.id !== change.id), change] : [];
      const removed = new Set(previous.filter(item => !approvals.some(next => next.id === item.id)).map(item => item.id));
      approvalOrder.current = approvalOrder.current.filter(key => !removed.has(key));
      for (const item of approvals) if (!approvalOrder.current.includes(item.id)) approvalOrder.current.push(item.id);
      updateRun({ approvals });
    };
    const setMessages = (change: SetStateAction<Message[]>) => {
      const next = typeof change === 'function' ? change(sessions.get(id ?? 'new').messages) : change;
      updateRun({ messages: next });
      if (activeIdRef.current === id) displayMessages(next);
    };
    updateRun({ messages, notice: '', error: '' });
    const streamedTools = new Set<string>();
    let retriedEarlyStop = false;

    // Chats live under a folder: a brand-new conversation needs one up front.
    const folderId = id ? projectId
      : (projectId && findProject(workspaceIndex, projectId) ? projectId : allProjects(workspaceIndex)[0]?.id) ?? null;

    if (!id && !folderId) {
      const message = 'Add a project folder first, then send.';
      setChatError(message);
      throw new Error(message);
    }

    const request = { cancelled: false, backendStarted: false };
    pendingSends.current.set(id ?? 'new', request);
    const ensurePending = () => { if (request.cancelled) throw new Error('Message cancelled before generation; it was not saved.'); };
    setPausedConversationId(null);
    setGenerating(true); setChatError(''); setLiveActivity({ state: 'Preparing', activity: 'Preparing request…' });

    const previousIds = new Set(messages.map(message => message.id));

    try {

      if (!id) {

        completionRefresh.current++;
        const conversation = await api.createConversation(folderId); id = conversation.id;
        completionRefresh.current++;
        sessions.move('new', id);
        pendingSends.current.delete('new'); pendingSends.current.set(id, request);
        // Creating a conversation may finish after the user has selected a
        // different one. The new run keeps its identity but must not seize the
        // visible session or move the new screen's draft in that case.
        if (creationSelection === selection.current && activeIdRef.current === null) {
          activeIdRef.current = id;
          setProjectId(folderId);
          try { moveNewDraft(id); } catch (e) { setError(errorMessage(e)); }
          draftId.current = id;
          setActiveId(id);
        }
        setData(current => ({ ...current, conversations: [conversation, ...current.conversations] }));
        ensurePending();
        setWorkspaceIndex(await api.saveTaskMeta(id, { projectId: folderId, archived: false, pinned: false }));
        ensurePending();

        // Persist the profile picked for this new chat before the turn is

        // assembled, so the backend renders the same payload the user chose.

        if (preset !== 'standard' && api.setPreset) {

          try { await api.setPreset(id, preset); } catch (e) { setError(errorMessage(e)); }

        }

      }

      ensurePending();
      await api.saveConversationTools(id, { sources: selectedConnectors, tools: selectedTools, accessMode });
      ensurePending();

      const conversationId = id;

      if (!resumeFailedMessageId) setMessages(current => [...current, { id: 'pending-user', conversationId, role: 'user', content, reasoning: '', status: 'complete', createdAt: Date.now() }]);

      request.backendStarted = true;
      await api.sendMessage(conversationId, content, event => {
        if (pendingSends.current.get(conversationId) !== request) return;
        if (request.cancelled && !retriedEarlyStop) {
          retriedEarlyStop = true;
          void api.cancelGeneration(conversationId).catch(() => {});
        }
        if (event.item) { publishTurnItem(event.item); return; }
        if (event.presentation) {
          setMessages(current => current.map(message => message.id === event.messageId ? { ...message, presentation: { ...event.presentation!, sourceText: event.presentation!.sourceText || message.presentation?.sourceText || message.content } } : message));
          return;
        }

        if (event.notice) { setChatNotice(event.notice); return; }

        if (event.context) updateRun({ context: event.context });

        if ('approval' in event) {
          const next = event.approval;
          // ID-scoped cleanup: a backend close notice dismisses only the
          // pending request it names, never a newer dialog.
          if (next && next.closed) setApproval(current => (current?.id === next.id ? null : current));
          else setApproval(next || null);
        }

        if (event.state || event.activity) {

          setLiveActivity({ state: event.state, activity: event.activity });

        }

        if (event.toolStream) {

          const stream = event.toolStream;

          const toolMsgId = `tool-${stream.toolCallId}`;
          streamedTools.add(toolMsgId);

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
            toolOutput.append(conversationId, toolMsgId, stream.stream === 'stderr' ? 'stderr' : 'stdout', stream.chunk ?? '');
            return;
          }

          if (stream.type === 'finished') {
            const output = toolOutput.read(conversationId, toolMsgId);

            setMessages(current =>

              current.map(m => {

                if (m.id !== toolMsgId) return m;

                let record: Record<string, unknown> = {};

                try { record = JSON.parse(m.content); } catch { /* ignore */ }

                const res = (record.result && typeof record.result === 'object' && !Array.isArray(record.result))

                  ? { ...(record.result as Record<string, unknown>) }

                  : {};

                res.stdout = output.stdout;
                res.stderr = output.stderr;
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

      }, selectedConnectors, selectedTools, planMode, reasoningOverride, undefined, recoveryId, resumeFailedMessageId);
      setChatNotice('');

    } catch (e) {

      setChatError(errorMessage(e));

      // Restore the draft only when persistence confirms the message was not accepted.

      // An inference failure can happen after the user message has already been saved.

      if (!id) throw Object.assign(new Error(errorMessage(e)), { conversationId: 'new' });

      let saved: Message[];

      try { saved = await api.messages(id); }

      catch {

        setChatError(`${errorMessage(e)} Could not verify whether your message was saved. Reopen the conversation before sending it again.`);

        return;

      }

      setMessages(saved);

      // A persisted prompt was accepted even when inference failed. The
      // explicit Retry control remains available; it is no longer an unsent draft.
      if (!resumeFailedMessageId && !saved.some(message => message.role === 'user' && !previousIds.has(message.id) && message.content === content.trim())) throw Object.assign(new Error(errorMessage(e)), { conversationId: id });

    }

    finally {

      try { if (id) setMessages(await api.messages(id)); }

      catch (e) {

        // Finalize any messages stuck in 'streaming' status so the UI doesn't

        // show perpetual spinners when the backend died mid-turn.

        setMessages(current => current.map(message => {
          if (message.status !== 'streaming') return message;
          let content = message.content;
          if (message.role === 'tool') {
            try {
              const record = JSON.parse(content);
              content = JSON.stringify({ ...record, result: { ...record.result, ...toolOutput.read(message.conversationId, message.id) } });
            } catch { /* Preserve legacy tool text on recovery failure. */ }
          }
          return { ...message, content, status: 'error' };
        }));

        setChatError(errorMessage(e));

      }

      const refreshVersion = ++completionRefresh.current;
      // Sidebar refresh is best-effort; a slow metadata request must not keep
      // the completed turn (and its composer) locked in the working state.
      void Promise.all([api.bootstrap(), api.workspaceIndex()]).then(([bootstrap, index]) => {
        if (refreshVersion === completionRefresh.current) {
          setData(normalizeBootstrap(bootstrap));
          setWorkspaceIndex(index);
        }
      }).catch(e => {
        if (refreshVersion === completionRefresh.current) setError(`Could not refresh conversations: ${errorMessage(e)}`);
      });

      if (id) for (const toolId of streamedTools) toolOutput.clear(id, toolId);
      if (pendingSends.current.get(id ?? 'new') === request) pendingSends.current.delete(id ?? 'new');
      setApproval(null);
      setGenerating(false);
      updateRun({ unread: activeIdRef.current !== id });

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

    catch (e) { setError(errorMessage(e)); setData(current => ({ ...current, runtime: { phase: 'error', message: errorMessage(e), modelPath: null } })); }

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

    const deleted: string[] = [];
    let failure = '';
    try {
      for (const id of ids) { await api.deleteConversation(id); deleted.push(id); }
    }
    catch (e) { failure = errorMessage(e); }
    finally {
      if (deleted.length) setData(current => ({ ...current, conversations: current.conversations.filter(item => !deleted.includes(item.id)) }));
      if (activeId && deleted.includes(activeId)) {
        selection.current++; setActiveId(null); setMessages([]); setRenaming(false);
        await newConversation();
      }
      if (failure) setError(failure);
    }

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

  const activeProject = allProjects(workspaceIndex).find(p => p.id === projectId);
  const activeProjectName = activeProject?.name ?? '';

  const settingsPage = page === 'general' ? <GeneralSettings accessMode={accessMode} onAccessMode={mode => void changeTools(selectedConnectors, selectedTools, mode)} busy={busy} nativeAvailable={nativeAvailable} projectPath={activeProject?.path} modelLabel={modelLabel} scopeLabel={active?.title} onPage={navigate} /> : page === 'appearance' ? <AppearanceSettings preferences={appearance.preferences} theme={theme} onChange={appearance.update} onReset={appearance.reset} saveError={appearance.saveError} /> : page === 'tools' ? <ToolsSettings onChooseWorkspace={chooseTaskWorkspace} accessMode={accessMode} onAccessModeChange={mode => void changeTools(selectedConnectors, selectedTools, mode)} preset={preset} onPresetChange={changePreset} selectedTools={selectedTools} onToolsChange={tools => void changeTools(selectedConnectors, tools)} selected={selectedConnectors} onChange={sources => void changeTools(sources, selectedTools)} scopeLabel={active?.title} onBack={() => navigate('chat')} busy={busy} loading={loading} saving={savingTools} onNavigate={target => navigate(target)} /> : ['models', 'inference', 'diagnostics'].includes(page) ? <Models runningConversations={runningConversations} key={page} initialTab={page === 'inference' ? 'generation' : page === 'diagnostics' ? 'diagnostics' : 'library'} onRefresh={async () => { setData(normalizeBootstrap(await api.bootstrap())); }} providers={data.providers} selection={modelSelection} requestedModel={requestedModel} onRequestedModelHandled={() => setRequestedModel(undefined)} config={data.config} preferences={data.preferences} runtime={data.runtime} busy={!nativeAvailable || globalBusy} onLoad={() => void modelAction(true)} onUnload={() => void modelAction(false)} onSaveSelection={saveModelSelection} onSaveConfig={async config => { try { await api.saveConfig(config); setData(normalizeBootstrap(await api.bootstrap())); } catch (e) { setError(errorMessage(e)); throw e; } }} onSavePreferences={async preferences => { try { await api.savePreferences(preferences); setData(normalizeBootstrap(await api.bootstrap())); } catch (e) { setError(errorMessage(e)); throw e; } }} /> : page === 'security' ? <SecuritySettings accessMode={accessMode} onAccessMode={mode => void changeTools(selectedConnectors, selectedTools, mode)} busy={busy} scope={active?.title} /> : page === 'git' ? <GitSettings root={activeProject?.path} onOpenReview={() => { navigate('chat'); setReviewRequest(value => value + 1); }} /> : page === 'weather' ? <WeatherSettings /> : page === 'web-search' ? <Execution key={page} section="web-search" /> : page === 'execution' ? <Execution key={page} /> : page === 'hooks' ? <HooksSettings /> : page === 'development-tools' ? <DevelopmentSettings /> : page === 'language-servers' ? <LanguageServerSettings root={activeProject?.path} /> : page === 'bash' || page === 'python' ? <NativeExecutionSettings key={page} kind={page} /> : page === 'browser-use' ? <AutomationSettings key={page} mode="browser" onConfigureExecution={() => navigate('execution')} /> : page === 'computer-use' ? <AutomationSettings key={page} mode="computer" /> : page === 'connectors' ? <Connectors /> : page === 'permission-rules' ? <PermissionRulesSettings /> : page === 'plugins' ? <PluginSettings /> : <Skills />;

  return <div className={`app desktop-app ${collapsed ? 'sidebar-collapsed' : ''}`}>

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
          disabled={navigationBusy || navigation.cursor <= 0}
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
          disabled={navigationBusy || navigation.cursor >= navigation.ids.length - 1}
          onClick={() => {
            const cursor = navigation.cursor + 1;
            setNavigation(n => ({ ...n, cursor }));
            void selectConversation(navigation.ids[cursor], true);
          }}
        >
          <ArrowRight size={16} />
        </button>
      </div>

      <details className="app-menu" name="application-menu"><summary>File</summary><div><button onClick={() => setPaletteOpen(true)}>Commands… <kbd>Ctrl+Shift+P</kbd></button><button disabled={navigationBusy} onClick={event => { newConversation(); event.currentTarget.closest('details')?.removeAttribute('open'); }}>New task <kbd>Ctrl+N</kbd></button><button disabled={!active || busy} onClick={() => void exportChat()}>Save a copy…</button></div></details>

      <details className="app-menu" name="application-menu"><summary>Edit</summary><div><button disabled={!active || busy} onClick={event => { if (active) { setTitle(active.title); setRenaming(true); } event.currentTarget.closest('details')?.removeAttribute('open'); }}>Rename task</button><button onClick={event => { searchConversations(); event.currentTarget.closest('details')?.removeAttribute('open'); }}>Search tasks <kbd>Ctrl+K</kbd></button></div></details>



      <details className="app-menu" name="application-menu"><summary>Help</summary><div className="shortcut-help"><strong>Workspace shortcuts</strong><span>Review <kbd>Ctrl+Shift+G</kbd></span><span>Terminal <kbd>Ctrl+`</kbd></span><span>Browser <kbd>Ctrl+T</kbd></span><span>Files <kbd>Ctrl+P</kbd></span></div></details>

      <div className="titlebar-drag-region" data-tauri-drag-region aria-hidden="true" />

      <WindowControls onError={setError} />
    </div>

    <TabDock page={page} onPage={navigate} theme={theme} onTheme={toggleTheme} />

    {!collapsed && <Sidebar workspace={workspaceIndex} onWorkspace={setWorkspaceIndex} projectId={projectId} onProject={id => void selectProject(id)} onDeleteConversations={ids => void removeConversations(ids)} onHit={(id, messageId) => { setHitId(messageId); void selectConversation(id); }} page={page} onPage={nextPage => nextPage === 'chat' ? goHome() : navigate(nextPage)} conversations={data.conversations} activeId={activeId} onSelect={id => void selectConversation(id)} onNew={targetProjectId => void newConversation(targetProjectId)} busy={globalBusy} navigationBusy={navigationBusy} sessionRuns={sessions.runs} pendingConversationIds={data.pendingConversationIds} search={search} onSearch={setSearch} theme={theme} onTheme={toggleTheme} />}

    {approval && <ApprovalDialog key={approval.id} sessionLabel={data.conversations.find(item => item.id === approvalEntry?.id)?.title} request={approval} onResolve={allow => api.resolveToolApproval(approval.id, allow)} onResolveAskUser={(choice) => api.resolveAskUser(approval.id, choice)} onResolveMcpInput={(action, content) => api.resolveMcpInput(approval.id, action, content)} />}

    <main className="workspace" id="main-content"><DownloadActivity /><CommandPalette open={paletteOpen} onClose={() => setPaletteOpen(false)} commands={[
      { id: 'plan', label: planMode ? 'Turn off plan before acting' : 'Plan before acting', disabled: busy, run: () => setPlanMode(!planMode) },
      { id: 'new', label: 'New conversation', disabled: navigationBusy, run: () => void newConversation() },
      { id: 'search', label: 'Search conversations', run: searchConversations },
      { id: 'sidebar', label: 'Toggle sidebar', run: () => setCollapsed(value => !value) },
      ...(['general', 'appearance', 'models', 'inference', 'tools', 'execution', 'security', 'connectors', 'skills', 'diagnostics'] as Page[]).map(target => ({ id: target, label: `Open ${target === 'inference' ? 'generation settings' : target}`, run: () => navigate(target) })),
      { id: 'theme', label: `Use ${theme === 'dark' ? 'light' : 'dark'} theme`, run: toggleTheme },
    ]} />



      <div className="workspace-split"><div className="workspace-conversation">
      <header className={`workspace-header ${!messages.length && !renaming ? 'workspace-header-minimal' : ''}`}><div className="workspace-lead"><button className="icon-button" aria-label="Back" disabled={navigationBusy || navigation.cursor <= 0} onClick={() => { const cursor = navigation.cursor - 1; setNavigation(n => ({ ...n, cursor })); void selectConversation(navigation.ids[cursor], true); }}><ArrowLeft size={17} /></button><button className="icon-button" aria-label="Forward" disabled={navigationBusy || navigation.cursor >= navigation.ids.length - 1} onClick={() => { const cursor = navigation.cursor + 1; setNavigation(n => ({ ...n, cursor })); void selectConversation(navigation.ids[cursor], true); }}><ArrowRight size={17} /></button><Folder className="task-folder-icon" size={18} />{renaming ? <form className="rename-form" onSubmit={event => { event.preventDefault(); void rename(); }}><input autoFocus aria-label="Conversation title" maxLength={160} value={title} onChange={e => setTitle(e.target.value)} /><button className="icon-button" aria-label="Save title"><Check size={16} /></button><button type="button" className="icon-button" aria-label="Cancel rename" onClick={() => setRenaming(false)}><X size={16} /></button></form> : <span className="workspace-title">{active?.title || 'New conversation'}</span>}</div><div className="header-actions">{page === 'chat' && allProjects(workspaceIndex).length > 0 && <select aria-label="Task project" value={projectId ?? allProjects(workspaceIndex)[0]?.id} disabled={busy} onChange={e => void selectProject(e.target.value)}>{allProjects(workspaceIndex).map(p => <option value={p.id} key={p.id}>{p.name}</option>)}</select>}{page === 'chat' && active && <details className="app-menu task-menu" name="application-menu"><summary aria-label="Task actions"><MoreHorizontal size={19} /></summary><div><button className="icon-button" title="Rename conversation" aria-label="Rename conversation" disabled={busy} onClick={() => { setTitle(active.title); setRenaming(true); }}><Pencil size={15} /></button><button className="icon-button" title="Export conversation" aria-label="Export conversation" disabled={!messages.length || busy} onClick={() => void exportChat()}><Download size={15} /></button><button className="icon-button" title="Delete conversation" aria-label="Delete conversation" disabled={busy} onClick={() => void removeConversation()}><Trash2 size={15} /></button></div></details>}<WorkspacePanel host={workspacePanelHost} reviewRequest={reviewRequest} onChooseWorkspace={chooseTaskWorkspace} busy={globalBusy} revision={`${projectId}:${activeId}`} visible={page === 'chat'} onReveal={() => navigate('chat')} /><button className="model-selector" disabled={busy} onClick={() => navigate('models')}><span className={`status-dot ${chatReady ? 'ready' : ''}`} />{modelLabel}<ChevronDown size={13} /></button></div></header>
      {!nativeAvailable && <div className="preview-banner">Browser preview · Open the desktop application to load models and save conversations.</div>}

      {exportedPath && <div className="preview-banner" role="status">Saved conversation to {exportedPath}<button className="icon-button" aria-label="Dismiss export confirmation" onClick={() => setExportedPath('')}><X size={16} /></button></div>}

      {error && <div className="error-banner" role="alert"><span>{error}</span><button className="icon-button" aria-label="Dismiss error" onClick={() => setError('')}><X size={16} /></button></div>}

      <Chat

        reasoningMode={reasoningOverride ?? data.preferences.reasoningMode}
        projectName={activeProjectName}

        attachmentDrafts={attachmentDrafts}

        onAttachmentDraftsChange={setAttachmentDrafts}

        conversationKey={activeId ?? 'new'}
        pendingTurn={savedPending?.conversationId === activeId ? savedPending : null}
        recoveryBusy={busy}
        onRecoverPending={() => {
          if (!savedPending || busy) return;
          const saved = savedPending;
          const revision = draftRevision.current;
          void send(saved.content, saved.id).then(() => {
            // A cancelled queued send can also have restored the same text to
            // the composer. Clear only that exact unchanged copy, after the
            // backend confirms the request was accepted.
            try {
              if (draftRevision.current === revision && loadDraft(saved.conversationId) === saved.content) {
                saveDraft(saved.conversationId, '');
                if (draftId.current === saved.conversationId && draftValue.current === saved.content) { draftValue.current = ''; setDraft(''); }
              }
            } catch (e) { setError(errorMessage(e)); }
          }).catch(e => setChatError(errorMessage(e)));
        }}
        onDiscardPending={() => {
          if (!savedPending || !activeId || busy) return;
          const saved = savedPending;
          void (async () => {
            if (!await confirm('Discard the saved unsent request? This cannot be undone.', { title: 'Discard saved request?', kind: 'warning' })) return;
            try {
              await api.discardPendingTurn(saved.conversationId, saved.id);
              setSavedPending(current => current?.id === saved.id ? null : current);
              setData(current => ({ ...current, pendingConversationIds: (current.pendingConversationIds ?? []).filter(id => id !== saved.conversationId) }));
            } catch (e) { setChatError(errorMessage(e)); }
          })();
        }}

        composerPermissions={<PermissionSelector accessMode={accessMode} onAccessModeChange={mode => void changeTools(selectedConnectors, selectedTools, mode)} busy={busy} nativeAvailable={nativeAvailable} />}

        contextUsage={contextUsage?.conversationId === activeId ? contextUsage.usage : undefined}

        modelLabel={modelLabel}
        modelIdentity={JSON.stringify([modelSelection, data.runtime.modelPath, data.config.contextLength, selectedRemoteModel])}

        selectionIssue={active?.providerSelectionRequired || (Boolean(modelSelection.providerId) && !remoteReady)}

        draft={draft}

        onDraftChange={changeDraft}
        onRestoreDraft={(owner, value) => {
          const key = owner === 'new' ? null : owner;
          try {
            const restored = loadDraft(key) || value;
            saveDraft(key, restored);
            if (draftId.current === key) { draftValue.current = restored; setDraft(restored); }
          } catch (e) { setError(errorMessage(e)); }
        }}

        messages={messages}

        generating={generating}

        ready={chatReady}

        loading={loading || checkingSessions}

        disabled={savingTools || exporting || Boolean(savedPending && savedPending.conversationId === activeId)}

        preset={preset}

        connectorIds={selectedConnectors}

        connectorTools={selectedTools}

        onSend={send}
        onContinueFailed={messageId => { void send('', undefined, messageId).catch(e => setChatError(errorMessage(e))); }}

        onCancel={() => {
          const request = pendingSends.current.get(activeId ?? 'new');
          if (request) request.cancelled = true;
          if (request?.backendStarted || (!request && activeId && recoveredRuns.current.has(activeId))) void api.cancelGeneration(activeId ?? undefined).catch(e => setChatError(errorMessage(e)));
        }}
        paused={!generating && pausedConversationId === activeId && activeId !== null}
        onResume={() => {
          if (!activeId || busy) return;
          void api.resumeGeneration(activeId).then(checkpoint => {
            setPausedConversationId(null);
            const pending = (checkpoint.pending ?? []).join(', ');
            return send(`Continue from the paused checkpoint. Do not repeat completed work. Pending: ${pending}.`);
          }).catch(e => setChatError(errorMessage(e)));
        }}

        onConfigure={() => navigate('models')}

        planMode={planMode}


        onConfigureLocalModel={filename => { setRequestedModel(filename); navigate('models'); }}

        liveActivity={liveActivity}

        providers={data.providers}

        onSelectModel={next => saveModelSelection(next).catch(() => {})}

        chatError={chatError}

        chatNotice={chatNotice}

        onDismissError={() => setChatError('')}

      />{activeId && <Trajectory

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

      />}

      </div><div className="workspace-panel-host" ref={setWorkspacePanelHost} /></div>
    </main>

    {page !== 'chat' && <SettingsModal page={page} onPage={navigate} onClose={() => navigate('chat')}>{settingsPage}</SettingsModal>}

  </div>;

}
