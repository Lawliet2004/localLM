import { localInferenceProvider } from '../lib/providerCapabilities';
import { type Dispatch, type SetStateAction, type ReactNode, memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { ArrowDown, ArrowUpRight, Check, ChevronDown, ChevronRight, Copy, Cpu, FileText, Folder, FolderGit2, ListTodo, MessageSquare, Plus, SendHorizontal, ShieldCheck, Square, Terminal, WandSparkles, X } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { attachmentAccept, composeMessage, readAttachment, type Attachment } from '../lib/attachments';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { modelLabel as localModelLabel } from '../lib/localModels';
import type { SessionEvent, Message, ContextUsage, ModelSelection, ProviderConnection, PreflightBreakdown, TaskWorkspace, ToolSelection, TurnMetrics, InteractionMode } from '../lib/types';
import type { InstalledModel } from '../lib/types';
import { ActivityTimeline, useSessionActivity } from './ActivityTimeline';
import { Evidence } from './Evidence';
import { ReadinessCard } from './Qualification';
import { TraceIcon } from './TraceIcon';
import { Segmented } from './Settings';
import { ContextControl } from './ContextControl';
import { CommandRunCard } from './CommandRunCard';
import { PlanChecklist, todosFromPlanEvents, useConversationTodos } from './PlanChecklist';
import type { RunRecord } from '../lib/types';

interface Props {
  composerTools?: ReactNode;
  conversationKey?: string;
  attachmentDrafts?: Record<string, Attachment[]>;
  onAttachmentDraftsChange?: Dispatch<SetStateAction<Record<string, Attachment[]>>>;
  contextUsage?: ContextUsage;
  modelLabel?: string;
  modelKey?: string;
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
  onSend: (content: string, modeOverride?: InteractionMode) => Promise<void>;
  /** Enter while a task runs: queue the draft as a follow-up task. */
  onQueue?: (content: string) => Promise<void>;
  /** Send the draft to the running task as a steering note. */
  onSteer?: (content: string) => Promise<void>;
  onCancel: () => void;
  onConfigure: () => void;
  /** Ask/Plan/Act for this conversation, enforced by the backend. */
  interactionMode?: InteractionMode;
  onInteractionModeChange?: (mode: InteractionMode) => void;
  /** The conversation's active isolated task workspace, if one exists. */
  taskWorkspace?: TaskWorkspace | null;
  /** Create an isolated task workspace for this conversation. */
  onIsolateTask?: () => void;
  /** A Plan turn completed; offer the Act follow-up that runs it. */
  planReady?: boolean;
  onImplementPlan?: () => void;
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
  const structuredResult = dataRecord && 'result' in dataRecord
    ? dataRecord.result
    : resultRecord && (typeof resultRecord.answer === 'string' || resultRecord.sources !== undefined || resultRecord.documents !== undefined)
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

  return (
    <article className="message message-tool" aria-label="tool message">
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
    </article>
  );
}

function MarkdownCodeBlock({ language, code }: { language: string; code: string }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 2000);
    } catch {
      /* ignore */
    }
  };

  useEffect(() => () => clearTimeout(timer.current), []);

  return (
    <div className="code-block-wrapper">
      <div className="code-block-header">
        <span className="code-block-lang">{language || 'code'}</span>
        <button
          type="button"
          className="code-block-copy-btn"
          onClick={() => void onCopy()}
          aria-label="Copy code block"
          title="Copy code"
        >
          {copied ? <Check size={12} className="text-success" /> : <Copy size={12} />}
          <span>{copied ? 'Copied!' : 'Copy'}</span>
        </button>
      </div>
      <pre className="code-block-pre">
        <code>{code}</code>
      </pre>
    </div>
  );
}

function ImageLightbox({ src, alt }: { src?: string; alt?: string }) {
  const [isOpen, setIsOpen] = useState(false);
  const preview = useRef<HTMLButtonElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (isOpen) close.current?.focus();
  }, [isOpen]);
  function closePreview() {
    setIsOpen(false);
    requestAnimationFrame(() => preview.current?.focus());
  }
  if (!src) return <span className="image-reference">Image: {alt || 'image'}</span>;
  return (
    <>
      <button ref={preview} type="button" className="image-preview-wrapper" onClick={() => setIsOpen(true)} aria-label={`Enlarge ${alt || 'image'}`}>
        <img src={src} alt={alt || 'Image preview'} className="image-preview-thumb" />
      </button>
      {isOpen && createPortal(
        <div className="image-lightbox-overlay" role="dialog" aria-modal="true" aria-label={alt || 'Image preview'}
          onClick={e => { if (e.target === e.currentTarget) closePreview(); }}
          onKeyDown={e => {
            if (e.key === 'Escape') closePreview();
            if (e.key === 'Tab') { e.preventDefault(); close.current?.focus(); }
          }}>
          <div className="image-lightbox-modal">
            <img src={src} alt={alt || 'Enlarged image'} className="lightbox-img" />
            <button
              ref={close}
              type="button"
              className="lightbox-close-btn"
              onClick={closePreview}
              aria-label="Close preview"
            >
              <X size={18} />
            </button>
          </div>
        </div>, document.body
      )}
    </>
  );
}

const perfSpeed = (v?: number | null) => v == null ? '—' : `${v >= 100 ? Math.round(v) : v.toFixed(1)} t/s`;
const perfSecs = (v?: number | null) => v == null ? '—' : `${v < 10 ? v.toFixed(2) : v.toFixed(1)} s`;

function MessageBody({ message, perf, variant = 'final' }: { message: Message; perf?: TurnMetrics; variant?: 'final' | 'step' }) {
  let attached: { name: string; imageUrl?: string; content?: string }[] = [];
  let displayContent = message.content;
  if (message.role === 'user') {
    try {
      const envelope = JSON.parse(message.content);
      if (envelope?.kind === 'locallm-attachments-v1' && typeof envelope.text === 'string' && Array.isArray(envelope.attachments)) {
        displayContent = envelope.text;
        attached = envelope.attachments.filter((file: unknown) => file && typeof file === 'object' && 'name' in file && typeof file.name === 'string');
      }
    } catch { /* Ordinary text message. */ }
  }
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState('');
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const startTime = useRef(Date.now());
  const isStreamingInitially = useRef(message.status === 'streaming');
  const [elapsed, setElapsed] = useState('0.0');
  const [finalDuration, setFinalDuration] = useState<string | null>(null);
  const [isOpen, setIsOpen] = useState(message.status === 'streaming');
  const userToggled = useRef(false);
  const hasReasoning = Boolean(message.reasoning);

  useEffect(() => () => clearTimeout(timer.current), []);

  useEffect(() => {
    if (!hasReasoning) return;
    if (message.status === 'streaming') {
      isStreamingInitially.current = true;
      setIsOpen(true);
      userToggled.current = false;
      const interval = setInterval(() => {
        const secs = ((Date.now() - startTime.current) / 1000).toFixed(1);
        setElapsed(secs);
      }, 1000);
      return () => clearInterval(interval);
    } else {
      if (isStreamingInitially.current && !finalDuration) {
        const secs = Math.max(0.5, (Date.now() - startTime.current) / 1000).toFixed(1);
        setFinalDuration(secs);
      }
      if (!userToggled.current) {
        setIsOpen(false);
      }
    }
  }, [message.status, hasReasoning]);

  const thoughtSummary = message.status === 'streaming'
    ? `Thinking (${elapsed}s)…`
    : finalDuration
    ? `Thought for ${Math.round(parseFloat(finalDuration)) || finalDuration}s`
    : 'Thought';

  return (
    <article data-message-id={message.id} className={`message message-${message.role}${variant === 'step' ? ' message-step' : ''}`} aria-label={`${message.role} message`}>
      <div className="message-byline">{message.role === 'user' ? 'You' : <><span className="mini-mark">L</span>LocalLM</>}</div>
      {message.reasoning && (
        <details
          className="reasoning trace-thought-details"
          open={isOpen}
          onToggle={e => {
            userToggled.current = true;
            setIsOpen((e.currentTarget as HTMLDetailsElement).open);
          }}
        >
          <summary className="trace-thought-pill">
            <TraceIcon kind="thought" size={12}>
              {message.status === 'streaming' ? <span className="thinking-pulsar" /> : undefined}
            </TraceIcon>
            <span className="trace-thought-label">{thoughtSummary}</span>
            <ChevronRight size={13} className="thought-chevron" />
          </summary>
          <div className="reasoning-content trace-thought-content">{message.reasoning}</div>
        </details>
      )}
      <div className={`markdown ${message.status === 'streaming' && displayContent ? 'is-streaming' : ''}`}>
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          components={{
            a: ({ children, href }) => <a href={href} target="_blank" rel="noreferrer">{children}</a>,
            img: ({ src, alt }) => <ImageLightbox src={src} alt={alt} />,
            code: ({ inline, className, children, ...rest }: any) => {
              const match = /language-(\w+)/.exec(className || '');
              const textContent = String(children).replace(/\n$/, '');
              if (!inline && (match || textContent.includes('\n'))) {
                return <MarkdownCodeBlock language={match ? match[1] : ''} code={textContent} />;
              }
              return <code className={className} {...rest}>{children}</code>;
            },
          }}
        >
          {displayContent}
        </ReactMarkdown>
        {attached.map((file, index) => <details key={index} className="message-attachment"><summary>{file.name}</summary>{file.imageUrl ? <img src={file.imageUrl} alt={file.name} style={{ maxWidth: '100%', maxHeight: 320 }} /> : <pre>{file.content}</pre>}</details>)}
      </div>
      {message.status === 'streaming' && !message.content && !message.reasoning && (
        <span className="thinking" role="status">Thinking<span>···</span></span>
      )}
      {(message.status === 'interrupted' || message.status === 'error') && (
        <p className="message-state">{message.error || (message.status === 'interrupted' ? 'Response stopped' : 'Response interrupted by an error')}</p>
      )}
      {variant === 'final' && message.content && message.status !== 'streaming' && (
        <div className="message-actions">
          <button
            className="icon-button copy-message"
            aria-label="Copy message"
            title="Copy message"
            onClick={async () => {
              try {
                await navigator.clipboard.writeText(message.role === 'user' ? displayContent : message.content);
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
          {perf && message.role === 'assistant' && message.status === 'complete' && (
            <p className="message-perf" title="Prompt processing speed · time to first token · token generation speed">
              PPS {perfSpeed(perf.promptTokPerSec)} · TTFT {perfSecs(perf.ttftSecs)} · TGS {perfSpeed(perf.genTokPerSec)}
            </p>
          )}
        </div>
      )}
      {copyError && <small role="alert">{copyError}</small>}
    </article>
  );
}

const sameItems = <T,>(a: T[], b: T[]) => a.length === b.length && a.every((item, i) => item === b[i]);
const ConversationTurn = memo(function ConversationTurn({ messages, events, run, onStop }: {
  messages: Message[]; events: SessionEvent[]; run: RunRecord | null; onStop: () => void;
}) {
  const user = messages.find(message => message.role === 'user');
  // `memory_used` events name exactly which facts the turn injected — shown
  // as a disclosure so recalled memory is visible, not silent.
  const usedMemories = events
    .filter(event => event.eventType === 'memory_used')
    .flatMap(event => {
      const facts = (event.payload as { facts?: { id: string; fact: string }[] }).facts;
      return Array.isArray(facts) ? facts : [];
    });
  return <div className="conversation-turn">
    {user && <MessageBody message={user} />}
    <ActivityTimeline compact events={events} messages={messages} run={run}
      renderMessage={(message, final) => <MessageBody message={message} variant={final ? 'final' : 'step'} perf={final ? message.metrics ?? undefined : undefined} />}
      renderTool={message => <ToolMessage message={message} onStop={onStop} />} />
    {usedMemories.length > 0 && <details className="memory-used">
      <summary>Used {usedMemories.length} {usedMemories.length === 1 ? 'memory' : 'memories'}</summary>
      <ul>{usedMemories.map(fact => <li key={fact.id}>{fact.fact}</li>)}</ul>
    </details>}
  </div>;
}, (previous, next) => previous.run === next.run && previous.onStop === next.onStop && sameItems(previous.messages, next.messages) && sameItems(previous.events, next.events));

const EMPTY_CONNECTORS: string[] = [];
const EMPTY_TOOLS: ToolSelection[] = [];
const EMPTY_ATTACHMENTS: Attachment[] = [];

export function Chat({
  attachmentDrafts: controlledAttachments,
  onAttachmentDraftsChange,
  composerTools,
  conversationKey = 'new',
  contextUsage,
  modelLabel = 'Local model',
  modelKey = modelLabel,
  selectionIssue = false,
  messages,
  generating,
  ready,
  loading,
  disabled = false,
  preset = 'standard',
  connectorIds = EMPTY_CONNECTORS,
  connectorTools = EMPTY_TOOLS,
  onSend,
  onQueue,
  onSteer,
  onCancel,
  onConfigure,
  interactionMode = 'act',
  onInteractionModeChange,
  taskWorkspace = null,
  onIsolateTask,
  planReady = false,
  onImplementPlan,
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
  const stopRef = useRef(onCancel);
  stopRef.current = onCancel;
  const stopGeneration = useCallback(() => stopRef.current(), []);
  const planMode = interactionMode === 'plan';
  const sessionActivity = useSessionActivity(conversationKey, generating);
  const planFallback = todosFromPlanEvents(sessionActivity);
  const todos = useConversationTodos(conversationKey, generating, planFallback);
  const completedPlan = todos.length > 0 && todos.every(todo => todo.status === 'completed');
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
  const [preflight, setPreflight] = useState<{ breakdown: PreflightBreakdown; draft: string } | null>(null);
  const composedDraft = useMemo(() => {
    try { return composeMessage(draft, attachments); } catch { return ''; }
  }, [draft, attachments]);
  const preflightRevision = useRef(0);
  const [contextRevision, setContextRevision] = useState(0);
  const [compacting, setCompacting] = useState(false);
  const [compactionNotice, setCompactionNotice] = useState<string | null>(null);
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
      setContextRevision(value => value + 1);
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
  const activityByTurn = useMemo(() => {
    const buckets: SessionEvent[][] = turns.map(() => []);
    const starts = turns.map(turn => turn.find(message => message.role === 'user')?.createdAt ?? 0);
    for (const event of sessionActivity) {
      // Binary search handles timestamps independent of event sequence order.
      let low = 0, high = starts.length;
      while (low < high) {
        const middle = (low + high) >>> 1;
        if (starts[middle] <= event.createdAt) low = middle + 1;
        else high = middle;
      }
      if (low > 0) buckets[low - 1].push(event);
    }
    return buckets;
  }, [turns, sessionActivity]);

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
    const revision = ++preflightRevision.current;
    setPreflight(null);
    if (!composedDraft.trim() || !ready || generating || loading || disabled) return;
    preflightTimer.current = setTimeout(() => {
      void api.contextPreflight(conversationKey === 'new' ? null : conversationKey, composedDraft, preset, connectorIds, connectorTools, false, interactionMode).then(result => {
        if (preflightRevision.current !== revision) return;
        setPreflight(result.available ? { breakdown: result.breakdown!, draft: composedDraft } : null);
      }).catch(() => { if (preflightRevision.current === revision) setPreflight(null); });
    }, 400);
    return () => {
      ++preflightRevision.current;
      if (preflightTimer.current) clearTimeout(preflightTimer.current);
    };
  }, [composedDraft, ready, generating, loading, disabled, messages, conversationKey, connectorIds, connectorTools, preset, interactionMode, modelKey, contextRevision]);

  async function submit() {
    if ((!draft.trim() && !attachments.length) || !ready || loading || disabled || reading.current || submitting.current) return;
    let content: string;
    try {
      content = composeMessage(draft, attachments);
    } catch (error) {
      setAttachmentError((error as Error).message);
      return;
    }
    if (generating) {
      // A task is already running: Enter queues the draft as a follow-up
      // instead of touching the running task.
      if (!onQueue) return;
      submitting.current = true;
      const originalDraft = draft;
      setDraft('');
      try {
        await onQueue(content);
      } catch {
        setDraft(current => current || originalDraft);
      } finally {
        submitting.current = false;
      }
      input.current?.focus();
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

  return (
    <div className="chat-layout" style={{ position: 'relative' }}>
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
              const recordedRun = user?.id === lastPrompt?.id && latestRun?.conversationId === conversationKey && latestRun.createdAt >= (user?.createdAt ?? 0) ? latestRun : null;
              return <ConversationTurn key={turn[0].id} messages={turn} events={activityByTurn[turnIndex]} run={recordedRun} onStop={stopGeneration} />;
            })}
            {!generating && lastResponse && <Evidence conversation={conversationKey} tick={messages.length} />}
            {generating && (
              <div className="activity-live" role="status">
                <span className="activity-dot" aria-hidden="true" />
                <span className="activity-live-text">{liveActivity?.activity || 'Generating response…'}</span>
              </div>
            )}
          </div>
        ) : (
          <div className="welcome">
              <h1>Start a local conversation</h1>
              {ready
                ? <p className="welcome-description">
                    {projectName ? `Ask a question or work on ${projectName}.` : 'Ask a question, attach a file, or make a plan.'}
                  </p>
                : <ReadinessCard onAction={onConfigure} />}
              <div className="suggestions">
                {[
                  { icon: MessageSquare, title: 'Think it through', prompt: 'Help me think through an idea. Start by asking me what I want to achieve.' },
                  { icon: Terminal, title: 'Write some code', prompt: 'Help me write a small, useful program. Ask me what it should do.' },
                  { icon: WandSparkles, title: 'Make a plan', prompt: 'Help me turn a goal into a practical plan. Ask me about my goal first.' },
                ].map(({ icon: Icon, title, prompt }) => (
                  <button
                    key={title}
                    onClick={() => {
                      setDraft(prompt);
                      input.current?.focus();
                    }}
                  >
                    <Icon size={16} aria-hidden="true" />
                    <span>{title}</span>
                    <ArrowUpRight size={14} className="suggestion-arrow" aria-hidden="true" />
                  </button>
                ))}
              </div>
            </div>
        )}
      </div>

      {showJumpToLatest && (
        <button
          type="button"
          className="jump-to-latest"
          onClick={() => {
            if (scroll.current) {
              scroll.current.scrollTo({ top: scroll.current.scrollHeight, behavior: 'smooth' });
              follow.current = true;
              setShowJumpToLatest(false);
            }
          }}
          aria-label="Jump to latest message"
        >
          <ArrowDown size={14} /> Latest
        </button>
      )}

      <div className="composer-region">
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
        {!ready && messages.length === 0 && (
          <div className="setup-hint">
            <Cpu size={15} />
            <span>{selectionIssue ? 'Select a local model or a tested loopback provider with configured model limits.' : 'Load a local model or choose a tested loopback provider to start a conversation.'}</span>
            <button onClick={onConfigure}>Open Models <ArrowUpRight size={13} aria-hidden="true" /></button>
          </div>
        )}
        <div className="codex-composer-wrapper">
          {!completedPlan && <PlanChecklist
            todos={todos}
            generating={generating}
            drafting={planMode && generating && todos.length === 0}
            conversationId={conversationKey}
          />}
          {planReady && !generating && onImplementPlan && (
            <button
              type="button"
              className="implement-plan"
              disabled={!ready || loading || disabled}
              onClick={() => onImplementPlan?.()}
            >
              <ListTodo size={14} aria-hidden="true" /> Implement plan
            </button>
          )}
          <div className="composer-meta-bar">
            <div className="composer-meta-item">
              <Folder size={13} className="composer-meta-icon" />
              <span>{projectName || 'No workspace'}</span>
            </div>
            {taskWorkspace
              ? <span className="composer-meta-item task-active" title={`Isolated task (${taskWorkspace.kind === 'git-worktree' ? 'git worktree' : 'snapshot copy'}) — review and apply changes in the Review tab`}>
                  <ShieldCheck size={13} className="composer-meta-icon" />
                  Isolated{taskWorkspace.branch ? ` · ${taskWorkspace.branch.split('/').pop()}` : ' copy'}
                </span>
              : onIsolateTask && <button
                  type="button"
                  className="composer-meta-item task-isolate"
                  aria-label="Isolate task"
                  title="Work in an isolated copy of this workspace so changes can be reviewed and applied later"
                  disabled={generating || loading || disabled}
                  onClick={onIsolateTask}
                >
                  <FolderGit2 size={13} className="composer-meta-icon" /> Isolate task
                </button>}
            {completedPlan && <div className="composer-plan-compact"><PlanChecklist todos={todos} conversationId={conversationKey} /></div>}
            {!ready && messages.length > 0 && <button type="button" className="composer-setup-link" onClick={onConfigure} title={selectionIssue ? 'Choose a loopback provider or select the local model' : 'Load a local model or choose a tested loopback provider'}><Cpu size={13} /> Set up model</button>}
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
            placeholder={planMode ? 'Describe the task to plan…' : 'Do anything'}
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
              {/* Ask/Plan/Act is backend-enforced and saved per conversation;
                  keep it visually separate from the permission selector. */}
              <Segmented<InteractionMode>
                label="Interaction mode"
                value={interactionMode}
                onChange={mode => onInteractionModeChange?.(mode)}
                options={[
                  { value: 'ask', label: 'Ask' },
                  { value: 'plan', label: 'Plan' },
                  { value: 'act', label: 'Act' },
                ]}
              />
              <div className="composer-tools">{composerTools}</div>
            </div>
            <div className="composer-send" style={{ position: 'relative' }}>
              <ContextControl
                conversationId={conversationKey}
                busy={generating || loading}
                usage={contextUsage}
                compacting={compacting}
                onCompact={messages.length > 2 ? () => void handleCompact() : undefined}
              />
              <button
                type="button"
                className="composer-model"
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
                <div className="composer-model-popover" role="dialog" aria-label="Select model">
                  <div className="composer-model-header">
                    <span>Select model</span>
                    <button type="button" className="icon-button" onClick={() => setShowModelPicker(false)}>
                      <X size={13} />
                    </button>
                  </div>
                  <div className="composer-model-list">
                    {libraryError && <p className="error" role="alert">{libraryError}</p>}
                    {onConfigureLocalModel && downloadedModels.filter(model => model.complete).map(model => <button type="button" className="composer-model-option" key={model.id} onClick={() => { setShowModelPicker(false); onConfigureLocalModel(model.id); }}><strong>{localModelLabel(model.filename)}</strong><small>{model.filename === 'K2-Horizon-4B-Q4_K_M.gguf' && model.external ? 'K2 custom llama.cpp runtime' : 'Local llama.cpp runtime'} · Use model</small></button>)}
                    <button
                      type="button"
                      className="composer-model-option"
                      onClick={() => {
                        void onSelectModel?.({ providerId: null, modelId: '' });
                        setShowModelPicker(false);
                      }}
                    >
                      <strong>Local model</strong>
                      <small>llama.cpp local runtime</small>
                    </button>
                    {providers?.filter(p => p.verified && localInferenceProvider(p)).map(p => (
                      <div key={p.id} className="composer-provider-group">
                        <span className="composer-provider-title">{p.name}</span>
                        {p.models.map(m => (
                          <button
                            key={m.id}
                            type="button"
                            className="composer-model-option"
                            onClick={() => {
                              void onSelectModel?.({ providerId: p.id, modelId: m.id });
                              setShowModelPicker(false);
                            }}
                          >
                            <strong>{m.id}</strong>
                            {m.contextLength && <small>{(m.contextLength / 1024).toFixed(0)}k context capacity</small>}
                          </button>
                        ))}
                      </div>
                    ))}
                  </div>
                  <div className="composer-model-footer">
                    <button
                      type="button"
                      onClick={() => {
                        setShowModelPicker(false);
                        onConfigure();
                      }}
                    >
                      Model settings…
                    </button>
                  </div>
                </div>
              )}

              {generating ? (
                <>
                  {onSteer && (
                    <button
                      type="button"
                      className="send-button send-steer"
                      aria-label="Steer the running task with this text"
                      title="Steer the running task with this text"
                      disabled={!draft.trim() || loading || disabled}
                      onClick={() => {
                        const text = draft.trim();
                        if (!text || !onSteer) return;
                        const original = draft;
                        setDraft('');
                        void onSteer(text).catch(() => setDraft(original));
                      }}
                    >
                      <WandSparkles size={14} aria-hidden="true" />
                      <span className="send-steer-label">Steer</span>
                    </button>
                  )}
                  <button type="button" className="send-button send-stop" aria-label="Stop response" onClick={onCancel}>
                    <Square size={15} fill="currentColor" />
                  </button>
                </>
              ) : (
                <button
                  className={`send-button ${(!draft.trim() && !attachments.length) ? 'sr-only' : ''}`}
                  type="submit"
                  aria-label="Send message"
                  disabled={(!draft.trim() && !attachments.length) || !ready || loading || disabled || readingFiles}
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
        {preflight?.breakdown.overflow && (
          <p className="composer-note" role="alert">
            <span className="context-overflow">{preflight.breakdown.overflow}</span>
          </p>
        )}
      </div>
    </div>
  );
}
