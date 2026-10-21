import { FileResult } from './FileResult';
import { partitionActivity } from '../lib/activity';
import { WorkSummary } from './WorkSummary';
import { AnswerSources, collectAnswerSources } from './AnswerSources';
import { ScreenshotPreview } from './ScreenshotPreview';
import { type Dispatch, type SetStateAction, type ReactNode, memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowDown, Check, ChevronDown, ChevronRight, Copy, Cpu, FileText, Folder, Play, Plus, SendHorizontal, Settings, Square, X } from 'lucide-react';
import { attachmentAccept, composeMessage, readAttachment, type Attachment } from '../lib/attachments';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { prefersReducedMotion } from '../lib/appearance';
import { isSubscriptionFormat, remoteModelReady } from '../lib/providers';
import { modelLabel as localModelLabel } from '../lib/localModels';
import type { Message, ContextUsage, ModelSelection, ProviderConnection, PreflightBreakdown, ToolSelection } from '../lib/types';
import { ResponseMetrics } from './ResponseMetrics';
import { ToolResultExtras } from './ToolResultExtras';
import type { InstalledModel } from '../lib/types';
import { ActivityTimeline, useSessionActivity } from './ActivityTimeline';
import { TraceIcon } from './TraceIcon';
import { ContextControl } from './ContextControl';
import { CommandRunCard } from './CommandRunCard';
import { PlanChecklist, todosFromPlanEvents, useConversationTodos } from './PlanChecklist';
import { CanvasPanel } from './CanvasPanel';
import { artifactVersions, extractArtifacts, findArtifact } from '../lib/canvas';
import type { RunRecord } from '../lib/types';
import { MarkdownView } from './MarkdownView';
import { MessageRenderer } from '../genui/MessageRenderer';
import { ResearchContext, type HostClaim } from '../genui/grounding';
import { usePresentationMode } from '../hooks/usePresentationMode';

const EMPTY_ATTACHMENTS: Attachment[] = [];
const EMPTY_SOURCES: string[] = [];
const EMPTY_TOOLS: ToolSelection[] = [];

interface Props {
  composerPermissions?: ReactNode;
  reasoningMode?: 'auto' | 'quick' | 'think';
  conversationKey?: string;
  attachmentDrafts?: Record<string, Attachment[]>;
  onAttachmentDraftsChange?: Dispatch<SetStateAction<Record<string, Attachment[]>>>;
  contextUsage?: ContextUsage;
  modelLabel?: string;
  modelIdentity?: string;
  selectionIssue?: boolean;
  draft?: string;
  onDraftChange?: (value: string | ((previous: string) => string)) => void;
  messages: Message[];
  generating: boolean;
  ready: boolean;
  loading: boolean;
  disabled?: boolean;
  preset?: string;
  connectorIds?: string[];
  connectorTools?: ToolSelection[];
  onSend: (content: string) => Promise<void>;
  onCancel: () => void;
  onResume?: () => void;
  paused?: boolean;
  onConfigure: () => void;
  planMode?: boolean;
  onConfigureLocalModel?: (filename: string) => void;
  liveActivity?: { state?: string; activity?: string } | null;
  providers?: ProviderConnection[];
  onSelectModel?: (selection: ModelSelection) => Promise<void>;
  chatError?: string;
  chatNotice?: string;
  onDismissError?: () => void;
  projectName?: string;
}

function ToolMessage({ message, onStop }: { message: Message; onStop?: () => void }) {
  let record: Record<string, unknown> = {};
  try {
    const value: unknown = JSON.parse(message.content);
    if (value && typeof value === 'object' && !Array.isArray(value)) record = value as Record<string, unknown>;
  } catch {
    /* Preserve legacy tool content below. */
  }
  const request = record.request && typeof record.request === 'object' ? (record.request as Record<string, unknown>) : record;
  const name = typeof request.name === 'string' ? request.name : 'Tool';
  const connector = typeof request.connector === 'string' ? request.connector : 'Connector';
  const localName = typeof request.localServerName === 'string' && request.localServerName ? request.localServerName : undefined;
  const result = record.result;
  const resultRecord = result !== null && typeof result === 'object' && !Array.isArray(result) ? (result as Record<string, unknown>) : null;
  const dataRecord = resultRecord && resultRecord.data && typeof resultRecord.data === 'object' && !Array.isArray(resultRecord.data) ? (resultRecord.data as Record<string, unknown>) : null;
  const logsRecord = dataRecord && dataRecord.logs && typeof dataRecord.logs === 'object' && !Array.isArray(dataRecord.logs) ? (dataRecord.logs as Record<string, unknown>) : null;
  // A tool result is either the `locallm.tool-envelope/1` envelope (whose
  // payload lives under `data`) or a legacy bare object. Both shapes are
  // read here so a result is never silently dropped from the transcript.
  const isStructured = (value: Record<string, unknown> | null) =>
    !!value && (typeof value.answer === 'string' || value.sources !== undefined || value.documents !== undefined);
  const structuredResult = dataRecord && 'result' in dataRecord
    ? dataRecord.result
    : isStructured(dataRecord)
    ? dataRecord
    : isStructured(resultRecord)
    ? resultRecord
    : undefined;
  const resultStatus = dataRecord && typeof dataRecord.resultStatus === 'string' ? dataRecord.resultStatus : undefined;
  const diff = resultRecord && typeof resultRecord.diff === 'string' ? (resultRecord.diff as string) : null;
  const stdout = typeof logsRecord?.stdout === 'string' ? logsRecord.stdout : resultRecord && typeof resultRecord.stdout === 'string' ? resultRecord.stdout : '';
  const stderr = typeof logsRecord?.stderr === 'string' ? logsRecord.stderr : resultRecord && typeof resultRecord.stderr === 'string' ? resultRecord.stderr : '';
  const exitCode = dataRecord && typeof dataRecord.exitCode === 'number' ? dataRecord.exitCode : resultRecord && typeof resultRecord.exitCode === 'number' ? resultRecord.exitCode : null;
  const durationMs = resultRecord && typeof resultRecord.durationMs === 'number' ? resultRecord.durationMs : undefined;
  const errorRecord = resultRecord && resultRecord.error && typeof resultRecord.error === 'object' && !Array.isArray(resultRecord.error) ? (resultRecord.error as Record<string, unknown>) : null;
  const error = typeof errorRecord?.message === 'string' ? errorRecord.message : resultRecord && typeof resultRecord.error === 'string' ? resultRecord.error : null;
  const errorCode = typeof errorRecord?.code === 'string' ? errorRecord.code : undefined;
  const envelopeStatus = resultRecord && typeof resultRecord.status === 'string' ? resultRecord.status : undefined;
  const isError = resultRecord !== null && resultRecord.isError === true;
  const failed = message.status === 'error' || isError;
  const decision = typeof request.decision === 'string' ? request.decision : undefined;
  const status = message.status === 'interrupted'
    ? 'interrupted'
    : decision === 'denied'
    ? 'failed'
    : decision === 'pending'
    ? 'awaiting_approval'
    : message.status === 'streaming'
    ? 'running'
    : failed
    ? 'failed'
    : 'completed';
  const command = typeof request.arguments === 'object' && request.arguments && 'command' in request.arguments
    ? String(request.arguments.command)
    : undefined;
  const code = typeof request.arguments === 'object' && request.arguments && 'code' in request.arguments
    ? String(request.arguments.code)
    : undefined;
  const language = typeof request.arguments === 'object' && request.arguments && 'language' in request.arguments
    ? String(request.arguments.language)
    : undefined;
  const cwd = typeof request.arguments === 'object' && request.arguments && 'cwd' in request.arguments
    ? String(request.arguments.cwd)
    : undefined;
  const artifactId = resultRecord && typeof resultRecord._artifactId === 'string' ? resultRecord._artifactId : null;
  const originalBytes = resultRecord && typeof resultRecord._originalBytes === 'number' ? resultRecord._originalBytes : null;
  const authorization = typeof request.authorization === 'string' ? request.authorization : undefined;

  const fallbackStdout = stdout || (!resultRecord && typeof result === 'string' ? result : (structuredResult ? '' : (resultRecord ? JSON.stringify(resultRecord, null, 2) : message.content)));
  const structuredJson = structuredResult !== undefined && structuredResult !== null
    ? (() => { try { return JSON.stringify(structuredResult, null, 2); } catch { return String(structuredResult); } })()
    : null;

  if (/^(read_file|view_file|file_read)$/.test(name) && status === 'completed' && typeof resultRecord?._artifactId !== 'string' && typeof resultRecord?.content === 'string') {
    const args = request.arguments as Record<string, unknown> | undefined;
    return <article className="message message-tool" aria-label="tool message"><FileResult path={String(resultRecord.path ?? args?.path ?? 'File')} content={resultRecord.content} totalLines={typeof resultRecord.totalLines === 'number' ? resultRecord.totalLines : undefined} sha256={typeof resultRecord.sha256 === 'string' ? resultRecord.sha256 : undefined} raw={record} /></article>;
  }

  return (
    <article className="message message-tool" aria-label="tool message">
      <ToolResultExtras name={name} result={resultRecord} conversationId={message.conversationId} />
      <CommandRunCard
        onStop={onStop}
        toolName={name}
        command={command}
        code={code}
        language={language}
        cwd={cwd}
        environment={localName ?? connector}
        localServerId={localName ? connector : undefined}
        status={status}
        decision={decision}
        isError={isError}
        exitCode={exitCode}
        stdout={fallbackStdout}
        stderr={stderr}
        diff={diff}
        durationMs={durationMs}
        error={errorCode ? `${errorCode}: ${error}` : error}
        resultStatus={resultStatus}
        envelopeStatus={envelopeStatus}
        structuredResult={structuredJson}
        arguments={request.arguments as Record<string, unknown> | undefined}
        artifactId={artifactId}
        originalBytes={originalBytes}
        authorization={authorization}
      />
      {Array.isArray(resultRecord?._screenshotArtifactIds) && resultRecord._screenshotArtifactIds.filter((id): id is string => typeof id === 'string' && /^art_[a-f0-9]{32}$/.test(id)).slice(0, 2).map(id => <ScreenshotPreview key={id} id={id} conversationId={message.conversationId} />)}
    </article>
  );
}

const MessageBody = memo(function MessageBody({ message, suppressReasoning = false, sourceLinks = '{}', onSendPrompt, onOpenCanvas }: { message: Message; suppressReasoning?: boolean; sourceLinks?: string; onSendPrompt?: (text: string) => void; onOpenCanvas?: (code: string, language: string) => void }) {
  let attached: { name: string; imageUrl?: string; content?: string }[] = [];
  let displayContent = message.content;
  if (message.role === 'user') {
    try { const envelope = JSON.parse(message.content); if (envelope.kind === 'locallm-attachments-v1') { displayContent = envelope.text; attached = envelope.attachments; } } catch { /* Ordinary text message. */ }
  }
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState('');
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [isOpen, setIsOpen] = useState(false);
  useEffect(() => () => clearTimeout(timer.current), []);

  const thoughtSummary = message.status === 'streaming' ? 'Thinking…' : 'Thought';
  const streaming = message.status === 'streaming';

  return (
    <article data-message-id={message.id} className={`message message-${message.role}`} aria-label={`${message.role} message`}>
      <div className="message-byline">{message.role === 'user' ? 'You' : <><span className="mini-mark">L</span>LocalLM</>}</div>
      {message.reasoning && !suppressReasoning && (
        <details
          className="reasoning trace-thought-details"
          open={isOpen}
          onToggle={e => {
            setIsOpen((e.currentTarget as HTMLDetailsElement).open);
          }}
        >
          <summary className="trace-thought-pill">
            <TraceIcon kind="thought" size={12}>
              {streaming ? <span className="thinking-pulsar" /> : undefined}
            </TraceIcon>
            <span className="trace-thought-label">{thoughtSummary}</span>
            <ChevronRight size={13} className="thought-chevron" />
          </summary>
          <div className="reasoning-content trace-thought-content"><MarkdownView content={message.reasoning} assistant={false} streaming={streaming} /></div>
        </details>
      )}
      <MessageRenderer
        message={{...message, content:displayContent}}
        assistant={message.role === 'assistant'}
        streaming={streaming}
        sourceLinks={sourceLinks}
        onSendPrompt={onSendPrompt}
        onOpenCanvas={onOpenCanvas}
      />
      {attached.length > 0 && <div className="markdown">{attached.map((file, index) => <details key={index} className="message-attachment"><summary>{file.name}</summary>{file.imageUrl ? <img src={file.imageUrl} alt={file.name} style={{ maxWidth: '100%', maxHeight: 320 }} /> : <pre>{file.content}</pre>}</details>)}</div>}
      {message.metrics && <ResponseMetrics message={message} />}
      {(message.status === 'interrupted' || message.status === 'error') && (
        <p className="message-state">{message.error || (message.status === 'interrupted' ? 'Response stopped' : 'Response interrupted by an error')}</p>
      )}
      {message.content && (
        <button
          className="icon-button copy-message"
          aria-label="Copy message"
          title="Copy message"
          onClick={async () => {
            try {
              await navigator.clipboard.writeText(message.content);
              setCopied(true);
              clearTimeout(timer.current);
              timer.current = setTimeout(() => setCopied(false), 2000);
            } catch {
              setCopyError('Could not access the clipboard. Select the text to copy it.');
            }
          }}
        >
          {copied ? <Check size={14} /> : <Copy size={14} />}
        </button>
      )}
      {copyError && <small role="alert">{copyError}</small>}
    </article>
  );
});

export function Chat({
  attachmentDrafts: controlledAttachments,
  onAttachmentDraftsChange,
  composerPermissions,
  reasoningMode,
  conversationKey = 'new',
  contextUsage,
  modelLabel = 'Local model',
  modelIdentity = modelLabel,
  selectionIssue = false,
  messages,
  generating,
  ready,
  loading,
  disabled = false,
  preset = 'standard',
  connectorIds = EMPTY_SOURCES,
  connectorTools = EMPTY_TOOLS,
  onSend,
  onCancel,
  onResume,
  paused = false,
  onConfigure,
  planMode = false,
  onConfigureLocalModel,
  draft: controlledDraft,
  onDraftChange,
  liveActivity,
  providers,
  onSelectModel,
  chatError,
  chatNotice,
  onDismissError,
  projectName,
}: Props) {
  const presentation = usePresentationMode(conversationKey);
  const sessionActivity = useSessionActivity(conversationKey, generating);
  const planFallback = todosFromPlanEvents(sessionActivity);
  const todos = useConversationTodos(conversationKey, generating, planFallback);
  const [localAttachments, setLocalAttachments] = useState<Record<string, Attachment[]>>({});
  const [latestRun, setLatestRun] = useState<RunRecord | null>(null);
  useEffect(() => {
    setLatestRun(null);
    if (generating || conversationKey === 'new' || !api.getConversationRun) return;
    let disposed = false;
    api.getConversationRun(conversationKey).then(run => { if (!disposed) setLatestRun(run); }).catch(() => { /* Older backends may not have run records. */ });
    return () => { disposed = true; };
  }, [conversationKey, generating, messages.length]);
  const attachmentDrafts = controlledAttachments ?? localAttachments;
  const setAttachmentDrafts = onAttachmentDraftsChange ?? setLocalAttachments;
  const latestConversation = useRef(conversationKey);
  latestConversation.current = conversationKey;
  const attachments = attachmentDrafts[conversationKey] ?? EMPTY_ATTACHMENTS;
  const [attachmentError, setAttachmentError] = useState('');
  useEffect(() => {
    setAttachmentError('');
    setDragging(false);
  }, [conversationKey]);
  const [readingFiles, setReadingFiles] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [showModelPicker, setShowModelPicker] = useState(false);
  const modelTrigger = useRef<HTMLButtonElement>(null);
  const modelPopover = useRef<HTMLDivElement>(null);
  function closeModelPicker() { setShowModelPicker(false); modelTrigger.current?.focus(); }
  useEffect(() => {
    if (!showModelPicker) return;
    modelPopover.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
    function dismiss(event: PointerEvent) {
      if (!modelPopover.current?.contains(event.target as Node) && !modelTrigger.current?.contains(event.target as Node)) setShowModelPicker(false);
    }
    function escape(event: KeyboardEvent) {
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closeModelPicker(); }
    }
    document.addEventListener('pointerdown', dismiss);
    document.addEventListener('keydown', escape);
    return () => { document.removeEventListener('pointerdown', dismiss); document.removeEventListener('keydown', escape); };
  }, [showModelPicker]);
  const [downloadedModels, setDownloadedModels] = useState<InstalledModel[]>([]);
  const [libraryError, setLibraryError] = useState('');
  useEffect(() => {
    if (!nativeAvailable || !showModelPicker) return;
    let disposed = false;
    void api.listInstalledModels().then(models => { if (!disposed) { setDownloadedModels(models); setLibraryError(''); } }).catch(e => { if (!disposed) setLibraryError(errorMessage(e)); });
    return () => { disposed = true; };
  }, [showModelPicker]);
  const [showJumpToLatest, setShowJumpToLatest] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const reading = useRef(false);

  async function attach(files: File[]) {
    if (reading.current || generating || loading || disabled || submitting.current) return;
    if (attachments.length + files.length > 8) {
      setAttachmentError('Attach up to 8 files per message.');
      return;
    }
    reading.current = true;
    setReadingFiles(true);
    setAttachmentError('');
    try {
      const added = await Promise.all(files.map(readAttachment));
      composeMessage(draft, [...attachments, ...added]);
      setAttachmentDrafts(current => ({ ...current, [conversationKey]: [...attachments, ...added] }));
    } catch (error) {
      if (latestConversation.current === conversationKey) {
        setAttachmentError(error instanceof Error ? error.message : 'Could not attach these files.');
      }
    } finally {
      reading.current = false;
      setReadingFiles(false);
    }
  }

  const [localDraft, setLocalDraft] = useState('');
  const draft = controlledDraft ?? localDraft;
  const setDraft = onDraftChange ?? setLocalDraft;
  const [preflightIssue, setPreflightIssue] = useState('');
  const [preflight, setPreflight] = useState<{ breakdown: PreflightBreakdown; draft: string } | null>(null);
  const [compacting, setCompacting] = useState(false);
  const [compactionNotice, setCompactionNotice] = useState<string | null>(null);
  const [preflightRevision, setPreflightRevision] = useState(0);
  const preflightTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const input = useRef<HTMLTextAreaElement>(null);
  const submitting = useRef(false);

  async function handleCompact() {
    if (conversationKey === 'new' || compacting || loading || generating) return;
    setCompacting(true);
    setCompactionNotice(null);
    try {
      const outcome = await api.compactConversation(conversationKey);
      setCompactionNotice(outcome.note);
      setPreflightRevision(value => value + 1);
    } catch (e) {
      setCompactionNotice(errorMessage(e));
    } finally {
      setCompacting(false);
    }
  }

  const lastPrompt = [...messages].reverse().find(message => message.role === 'user');
  const lastResponse = [...messages].reverse().find(message => message.role === 'assistant');
  const lastTool = [...messages].reverse().find(message => message.role === 'tool');
  const canRetry = lastPrompt && lastResponse && (
    ['error', 'interrupted'].includes(lastResponse.status) ||
    (lastTool && ['error', 'streaming'].includes(lastTool.status))
  );
  const turns = useMemo(() => {
    const grouped: Message[][] = [];
    for (const message of messages) {
      if (message.role === 'user' || !grouped.length) grouped.push([]);
      grouped[grouped.length - 1].push(message);
    }
    return grouped;
  }, [messages]);
  const turnActivity = useMemo(() => partitionActivity(turns, sessionActivity), [turns, sessionActivity]);


  // Buttons rendered by generated UI submit a follow-up like a typed message.
  const sendFromUi = useCallback((text: string) => {
    if (!text.trim() || !ready || generating || loading || disabled || submitting.current) return;
    void onSend(text);
  }, [ready, generating, loading, disabled, onSend]);

  // Canvas: every fenced artifact in the conversation, grouped so later
  // blocks with the same title supersede earlier versions.
  const canvasArtifacts = useMemo(() => extractArtifacts(messages), [messages]);
  const canvasArtifactsRef = useRef(canvasArtifacts);
  canvasArtifactsRef.current = canvasArtifacts;
  const [canvasSel, setCanvasSel] = useState<{ group: string; version: number } | null>(null);
  const canvasVersions = canvasSel ? artifactVersions(canvasArtifacts, canvasSel.group) : [];
  const canvasArtifact = canvasSel && canvasVersions.length
    ? canvasVersions[Math.min(canvasSel.version, canvasVersions.length) - 1]
    : null;
  // Switching conversations closes the canvas — its artifacts belong to this thread.
  useEffect(() => { setCanvasSel(null); }, [conversationKey]);
  // Stable identity so memoized message bodies are not re-parsed per turn.
  const openCanvas = useCallback((code: string, language: string) => {
    const artifact = findArtifact(canvasArtifactsRef.current, code, language);
    if (artifact) setCanvasSel({ group: artifact.groupKey, version: artifact.version });
  }, []);

  async function retry() {
    if (!lastPrompt || !ready || generating || loading || disabled || submitting.current) return;
    submitting.current = true;
    try {
      await onSend(lastPrompt.content);
    } catch {
      /* App retains and displays the send error. */
    } finally {
      submitting.current = false;
    }
  }

  useEffect(() => {
    if (follow.current && scroll.current) {
      scroll.current.scrollTop = scroll.current.scrollHeight;
    }
  }, [messages, generating, liveActivity?.activity]);

  useEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight}px`;
  }, [draft, conversationKey]);

  useEffect(() => {
    let disposed = false;
    setPreflight(null); setPreflightIssue('');
    if ((!draft.trim() && !attachments.length) || !ready) return;
    let payload: string;
    try { payload = composeMessage(draft, attachments); } catch { return; }
    preflightTimer.current = setTimeout(() => {
      void api.contextPreflight(conversationKey === 'new' ? null : conversationKey, payload, preset, connectorIds, connectorTools, planMode, reasoningMode, presentation.effectiveMode).then(result => {
        if (!disposed) { setPreflight(result.available ? { breakdown: result.breakdown!, draft: payload } : null); setPreflightIssue(!result.available && result.reason?.includes('at most 64 tools') ? result.reason : ''); }
      }).catch(() => { if (!disposed) setPreflight(null); });
    }, 400);
    return () => { disposed = true; if (preflightTimer.current) clearTimeout(preflightTimer.current); };
  }, [draft, attachments, ready, conversationKey, connectorIds, connectorTools, preset, planMode, reasoningMode, modelIdentity, preflightRevision, presentation.effectiveMode]);

  async function submit() {
    if ((!draft.trim() && !attachments.length) || !ready || generating || loading || disabled || preflightIssue || reading.current || submitting.current) return;
    let content: string;
    try {
      content = composeMessage(draft, attachments);
    } catch (error) {
      setAttachmentError((error as Error).message);
      return;
    }
    submitting.current = true;
    const originalDraft = draft;
    setAttachmentError('');
    setAttachmentDrafts(current => ({ ...current, [conversationKey]: [] }));
    setDraft('');
    try {
      await onSend(content);
    } catch {
      setDraft(current => current || originalDraft);
      setAttachmentDrafts(current => ({ ...current, [conversationKey === 'new' ? latestConversation.current : conversationKey]: attachments }));
    } finally {
      submitting.current = false;
    }
    input.current?.focus();
  }

  const isHome = !loading && !messages.length && !generating;

  return (
    <div className="chat-shell">
    <div className={`chat-layout ${isHome ? 'chat-home' : ''}`} style={{ position: 'relative' }}>
      <div
        className="chat-scroll"
        ref={scroll}
        onScroll={() => {
          const element = scroll.current;
          if (element) {
            const isNearBottom = element.scrollHeight - element.scrollTop - element.clientHeight < 120;
            follow.current = isNearBottom;
            setShowJumpToLatest(!isNearBottom);
          }
        }}
      >
        {loading ? (
          <p className="loading-state" role="status">Opening conversation…</p>
        ) : messages.length || generating ? (
          <div className="messages">
            {turns.map((turn, turnIndex) => {
              const user = turn.find(message => message.role === 'user');
              const activity = turnActivity[turnIndex];
              const evidenceSources = collectAnswerSources(turn, activity);
              const hostClaims: HostClaim[] = activity.filter(event => event.eventType === "verification").flatMap(event => {
                const claims = (event.payload as {claims?:unknown[]})?.claims;
                return Array.isArray(claims) ? claims.slice(0,100).flatMap(raw => { if (!raw || typeof raw !== "object" || Array.isArray(raw)) return []; const claim = raw as {claim?:unknown;status?:unknown;sources?:unknown}; return typeof claim.claim === "string" && typeof claim.status === "string" ? [{claim:claim.claim.slice(0,4000),verdict:claim.status,citations:Array.isArray(claim.sources)?claim.sources.filter((id): id is string => typeof id === "string").flatMap(reference => evidenceSources.find(source => source.url === reference)?.ids ?? [reference]).slice(0,32):[]}] : []; }) : [];
              });
              const sourceLinks = JSON.stringify(Object.fromEntries(evidenceSources.flatMap(source => source.ids.map(id => [id, source.url]))));
              return <div key={turn[0].id} className="conversation-turn">
                {user && <MessageBody message={user} />}
                <ResearchContext.Provider value={{sources:evidenceSources,claims:hostClaims}}><WorkSummary messages={turn} events={activity} run={turnIndex === turns.length - 1 ? latestRun : undefined}><ActivityTimeline events={activity} messages={turn} generating={generating && turnIndex === turns.length - 1} renderMessage={m => <MessageBody message={m} suppressReasoning sourceLinks={sourceLinks} onSendPrompt={sendFromUi} onOpenCanvas={openCanvas} />} renderTool={m => <ToolMessage message={m} onStop={onCancel} />} /></WorkSummary></ResearchContext.Provider>
                <AnswerSources messages={turn} events={activity} />
                {turnIndex === turns.length - 1 && latestRun && ['failed', 'outcome_unknown', 'paused', 'cancelled'].includes(latestRun.status) && <p role="status" className="run-outcome">Run {latestRun.status.replace(/_/g, ' ')}. Review the activity above before retrying.</p>}
              </div>;
            })}
            {generating && (
              <div className="activity-live" role="status">
                <span className="activity-dot" />
                <span className="activity-live-text">{liveActivity?.activity || 'Generating response…'}</span>
                {liveActivity?.state && <span className="activity-badge">{liveActivity.state}</span>}
              </div>
            )}
          </div>
        ) : (
          <>
            <h1 className="sr-only">New conversation</h1>
            <div className="welcome" aria-hidden="true">
              <svg viewBox="0 0 512 512" width="44" height="44">
                <rect width="512" height="512" rx="112" fill="#191b22" />
                <path d="M154 126h64v230h146v58H154z" fill="#eceef2" />
                <circle cx="339" cy="155" r="42" fill="#3ec98d" />
              </svg>
            </div>
          </>
        )}
      </div>

      <div className="composer-region">
      {showJumpToLatest && (
        <button
          type="button"
          className="jump-to-latest"
          onClick={() => {
            if (scroll.current) {
              scroll.current.scrollTo({ top: scroll.current.scrollHeight, behavior: prefersReducedMotion() ? 'instant' : 'smooth' });
              follow.current = true;
              setShowJumpToLatest(false);
            }
          }}
          aria-label="Jump to latest message"
        >
          <ArrowDown size={14} /> Latest
        </button>
      )}


        {chatNotice && <div className="setup-hint" role="status">{chatNotice}</div>}
        {chatError && (
          <div className="error-banner" role="alert" style={{ margin: '8px 0' }}>
            <span>{chatError}</span>
            {onDismissError && (
              <button type="button" className="icon-button" onClick={onDismissError} aria-label="Dismiss error">
                <X size={16} />
              </button>
            )}
          </div>
        )}
        {canRetry && !generating && (
          <div className="setup-hint">
            <span>Retry sends the last prompt as a new message. Enabled tool actions may run again.</span>
            <button disabled={!ready || loading || disabled} onClick={() => void retry()}>Retry last prompt</button>
          </div>
        )}
        {!ready && (
          <div className="setup-hint">
            <Cpu size={15} />
            <span>{selectionIssue ? 'Choose a tested provider and configure its model limits, or select the local model.' : isHome ? 'Choose a model to get started.' : 'Load a local model or choose a tested API provider to start a conversation.'}</span>
            <button onClick={onConfigure}>Open Models <span>↗</span></button>
          </div>
        )}
        <div className="codex-composer-wrapper">
          <PlanChecklist
            todos={todos}
            generating={generating}
            drafting={planMode && generating && todos.length === 0}
            conversationId={conversationKey}
          />
          <div className="composer-meta-bar">
            <div className="composer-meta-item">
              <Folder size={14} className="composer-meta-icon" />
              <span>{projectName || 'No workspace'}</span>
            </div>
            <button type="button" className="icon-button composer-tray-settings" aria-label="Model settings" title="Models & runtime" onClick={onConfigure}><Settings size={15} /></button>
          </div>
          <form
            className={`composer codex-composer-body ${dragging ? 'composer-dragging' : ''}`}
            onDragOver={event => {
              event.preventDefault();
              if (event.dataTransfer.types.includes('Files')) setDragging(true);
            }}
            onDragLeave={event => {
              if (!event.currentTarget.contains(event.relatedTarget as Node)) setDragging(false);
            }}
            onDrop={event => {
              event.preventDefault();
              setDragging(false);
              void attach(Array.from(event.dataTransfer.files));
            }}
            onSubmit={event => {
              event.preventDefault();
              void submit();
            }}
          >
            <input
              ref={fileInput}
              className="attachment-input"
              type="file"
              aria-label="Attach files"
              accept={attachmentAccept}
              multiple
              disabled={readingFiles || generating || loading || disabled}
              onChange={event => {
                void attach(Array.from(event.target.files ?? []));
                event.target.value = '';
              }}
            />
            {attachments.length > 0 && (
              <div className="attachment-list" aria-label="Attached files">
                {attachments.map(file => (
                  <div className="attachment-chip" key={file.id}>
                    <FileText size={18} />
                    {file.imageUrl && <img src={file.imageUrl} alt="" width={32} height={32} />}
                    <span>
                      <strong>{file.name}</strong>
                      <small>{Math.max(1, Math.ceil(file.size / 1024))} KB · {file.imageUrl ? 'Image' : 'Text'}</small>
                    </span>
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={`Remove ${file.name}`}
                      disabled={readingFiles}
                      onClick={() =>
                        setAttachmentDrafts(current => ({
                          ...current,
                          [conversationKey]: attachments.filter(item => item.id !== file.id),
                        }))
                      }
                    >
                      <X size={14} />
                    </button>
                  </div>
                ))}
              </div>
            )}
            {attachmentError && <p className="attachment-error" role="alert">{attachmentError}</p>}
          <textarea
            ref={input}
            aria-label="Message"
            placeholder={planMode ? 'Describe the task to plan…' : 'Ask a question, or describe a task…'}
            value={draft}
            rows={1}
            maxLength={100000}
            onChange={e => setDraft(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                void submit();
              }
            }}
            onPaste={event => {
              const items = event.clipboardData?.files;
              if (!items?.length) return;
              const images = Array.from(items).filter(f => f.type.startsWith('image/'));
              if (!images.length) return;
              event.preventDefault();
              void attach(images);
            }}
          />
          <div className="composer-bottom">
            <div className="composer-actions">
              <button
                type="button"
                className="icon-button attach-button"
                aria-label="Add files"
                title="Attach images, text, code, or CSV files"
                disabled={readingFiles || generating || loading || disabled}
                onClick={() => fileInput.current?.click()}
              >
                <Plus size={20} />
              </button>
              {composerPermissions}
            </div>
            <div className="composer-send" style={{ position: 'relative' }}>
              {(!isHome || contextUsage) && <ContextControl
                conversationId={conversationKey}
                busy={generating || loading}
                usage={contextUsage}
                compacting={compacting}
                onCompact={messages.length > 2 ? () => void handleCompact() : undefined}
              />}
              <button
                type="button"
                ref={modelTrigger}
                className="composer-model"
                aria-expanded={showModelPicker}
                aria-haspopup="dialog"
                onClick={() => {
                  if (providers && onSelectModel) {
                    setShowModelPicker(prev => !prev);
                  } else {
                    onConfigure();
                  }
                }}
                title="Choose model"
                disabled={generating || loading}
              >
                <span className={`status-dot ${ready ? 'ready' : ''}`} />
                <span>{modelLabel}</span>
                <ChevronDown size={13} />
              </button>

              {showModelPicker && (
                <div ref={modelPopover} className="composer-model-popover" role="dialog" aria-label="Select model">
                  <div className="composer-model-header">
                    <span>Select model</span>
                    <button type="button" className="icon-button" aria-label="Close model picker" onClick={closeModelPicker}>
                      <X size={13} />
                    </button>
                  </div>
                  <div className="composer-model-list">
                    {libraryError && <p className="error" role="alert">{libraryError}</p>}
                    {onConfigureLocalModel && downloadedModels.filter(model => model.complete).map(model => <button type="button" className="composer-model-option" key={model.id} onClick={() => { closeModelPicker(); onConfigureLocalModel(model.id); }}><strong>{localModelLabel(model.filename)}</strong><small>{model.repo ?? "Local GGUF"} · Use model</small></button>)}
                    <button
                      type="button"
                      className="composer-model-option"
                      onClick={() => {
                        void onSelectModel?.({ providerId: null, modelId: '' });
                        closeModelPicker();
                      }}
                    >
                      <strong>Local model</strong>
                      <small>llama.cpp local runtime</small>
                    </button>
                    {providers?.filter(p => p.verified).map(p => (
                      <div key={p.id} className="composer-provider-group">
                        <span className="composer-provider-title">{p.name}</span>
                        {p.models.map(m => {
                          const ready = remoteModelReady(p, m);
                          return <button
                            key={m.id}
                            type="button"
                            className="composer-model-option"
                            disabled={!ready}
                            title={ready ? undefined : isSubscriptionFormat(p.apiFormat) ? 'Subscription providers are not selectable in this build' : !m.contextLength ? 'Set a context capacity in Models & runtime to use this model' : 'Add an API key in Models & runtime to use this model'}
                            onClick={() => {
                              void onSelectModel?.({ providerId: p.id, modelId: m.id });
                              closeModelPicker();
                            }}
                          >
                            <strong>{m.id}</strong>
                            <small>{m.contextLength ? `${(m.contextLength / 1024).toFixed(0)}k context capacity` : 'Context not configured'}</small>
                          </button>;
                        })}
                      </div>
                    ))}
                  </div>
                  <div className="composer-model-footer">
                    <button
                      type="button"
                      onClick={() => {
                        closeModelPicker();
                        onConfigure();
                      }}
                    >
                      Configure in Models & runtime ↗
                    </button>
                  </div>
                </div>
              )}

              {paused && !generating && onResume && <button type="button" className="send-button" aria-label="Resume response" onClick={onResume}><Play size={15} fill="currentColor" /></button>}
              {generating ? (
                <>
                  <button type="button" className="send-button send-stop" aria-label="Stop response" onClick={onCancel}><Square size={15} fill="currentColor" /></button>
                </>
              ) : (
                <button
                  className={`send-button ${(!draft.trim() && !attachments.length) ? 'sr-only' : ''}`}
                  type="submit"
                  aria-label="Send message"
                  disabled={(!draft.trim() && !attachments.length) || !ready || loading || disabled || readingFiles || Boolean(preflightIssue)}
                >
                  <SendHorizontal size={17} />
                </button>
              )}
            </div>
          </div>
        </form>
        </div>
        {compactionNotice && (
          <p className="composer-note compaction-notice" role="status">
            {compactionNotice}
          </p>
        )}
        {preflightIssue && <p className="composer-note" role="alert">{preflightIssue}</p>}
        {preflight?.breakdown.presentation && <details className="composer-note"><summary>Visual response budget</summary><p>Separate formatting request: at least {preflight.breakdown.presentation.minimumInputTokens.toLocaleString()} input tokens plus the original answer, with {preflight.breakdown.presentation.responseReserve.toLocaleString()} output tokens reserved. The full request is checked before generation; text remains available if it does not fit.</p></details>}
        {preflight?.breakdown.overflow && (
          <p className="composer-note" role="alert">
            <span className="context-overflow">{preflight.breakdown.overflow}</span>
          </p>
        )}
      </div>
    </div>
    {canvasArtifact && (
      <CanvasPanel
        artifact={canvasArtifact}
        versionCount={canvasVersions.length}
        onVersion={version => setCanvasSel({ group: canvasArtifact.groupKey, version })}
        onClose={() => setCanvasSel(null)}
        onFollowUp={sendFromUi}
      />
    )}
    </div>
  );
}
