import { useOwnedProcess } from '../lib/ownedProcess';
import { useToolOutput } from '../lib/toolOutput';
import { toolRecord } from '../lib/toolActivity';
import { sumMetric } from '../lib/responseMetrics';
import { FileResult } from './FileResult';
import { partitionActivity } from '../lib/activity';
import { WorkSummary } from './WorkSummary';
import { AnswerSources, useAnswerEvidence, answerSourceLinks } from './AnswerSources';
import { ScreenshotPreview } from './ScreenshotPreview';
import { type Dispatch, type SetStateAction, type ReactNode, memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowDown, Check, ChevronDown, Copy, Cpu, FileText, Folder, Play, Plus, SendHorizontal, Settings, Square, X } from 'lucide-react';
import { attachmentAccept, composeMessage, readAttachment, type Attachment } from '../lib/attachments';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { prefersReducedMotion } from '../lib/appearance';
import { isSubscriptionFormat, remoteModelReady } from '../lib/providers';
import { modelLabel as localModelLabel } from '../lib/localModels';
import type { Message, ContextUsage, ModelSelection, ProviderConnection, PreflightBreakdown, ToolSelection } from '../lib/types';
import { ResponseMetrics } from './ResponseMetrics';
import { ToolResultExtras } from './ToolResultExtras';
import type { InstalledModel } from '../lib/types';
import { ActivityTimeline, activityTools, useSessionActivity } from './ActivityTimeline';
import { ActivityIndicator } from './ActivityIndicator';
import { ReasoningDisclosure } from './ReasoningDisclosure';
import { ContextControl } from './ContextControl';
import { CommandRunCard } from './CommandRunCard';
import { isPlanStateTool, PlanChecklist, todosFromPlanEvents, useConversationTodos } from './PlanChecklist';
import { AnswerNavigation } from './AnswerNavigation';
import { CanvasPanel } from './CanvasPanel';
import { artifactVersions, extractArtifacts, findArtifact } from '../lib/canvas';
import type { PendingTurn, RunRecord, SessionEvent, TaskOutcome } from '../lib/types';
import { MessageRenderer } from '../genui/MessageRenderer';
import { ResearchContext, type HostClaim } from '../genui/grounding';
import { usePresentationMode } from '../hooks/usePresentationMode';

const ACTIVITY_STATE_LABELS: Record<string, string> = {
  Preparing: 'Preparing',
  ExecutingTools: 'Using tools',
  queued: 'Queued',
  generating: 'Writing',
  awaiting_approval: 'Needs approval',
  Streaming: 'Writing',
};

function activityStateLabel(state: string) {
  return ACTIVITY_STATE_LABELS[state] ?? state.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/_/g, ' ');
}

const EMPTY_ATTACHMENTS: Attachment[] = [];
const EMPTY_SOURCES: string[] = [];
const EMPTY_TOOLS: ToolSelection[] = [];

interface Props {
  composerPermissions?: ReactNode;
  reasoningMode?: 'auto' | 'quick' | 'think';
  conversationKey?: string;
  draftOwner?: string;
  draftProjectId?: string | null;
  pendingTurn?: PendingTurn | null;
  recoveryBusy?: boolean;
  onRecoverPending?: () => void;
  onDiscardPending?: () => void;
  attachmentDrafts?: Record<string, Attachment[]>;
  onAttachmentDraftsChange?: Dispatch<SetStateAction<Record<string, Attachment[]>>>;
  contextUsage?: ContextUsage;
  modelLabel?: string;
  modelIdentity?: string;
  selectionIssue?: boolean;
  draft?: string;
  onDraftChange?: (value: string | ((previous: string) => string)) => void;
  onRestoreDraft?: (conversationId: string, value: string) => void;
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
  onResume?: (taskId?: string) => void;
  onContinueFailed?: (messageId: string) => void;
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

const ToolMessage = memo(function ToolMessage({ message, onStop }: { message: Message; onStop?: () => void }) {
  const output = useToolOutput(message.conversationId, message.id);
  const record: Record<string, unknown> = toolRecord(message);
  const request = record.request && typeof record.request === 'object' ? (record.request as Record<string, unknown>) : record;
  const name = typeof request.name === 'string' ? request.name : 'Tool';
  const connector = typeof request.connector === 'string' ? request.connector : 'Connector';
  const displayConnector = connector === 'Harness' ? 'Local' : connector;
  const localName = typeof request.localServerName === 'string' && request.localServerName ? request.localServerName : undefined;
  const result = record.result;
  const resultRecord = result !== null && typeof result === 'object' && !Array.isArray(result) ? (result as Record<string, unknown>) : null;
  const dataRecord = resultRecord && resultRecord.data && typeof resultRecord.data === 'object' && !Array.isArray(resultRecord.data) ? (resultRecord.data as Record<string, unknown>) : null;
  const payload = dataRecord ?? resultRecord;
  const processData = payload?.process && typeof payload.process === 'object' ? payload.process as Record<string, unknown> : payload;
  const processId = ['process_start', 'shell_run', 'process_poll', 'process_write', 'process_stop', 'check_run', 'check_report', 'verification_run', 'verification_report'].includes(name)
    ? typeof processData?.id === 'string' ? processData.id : typeof payload?.processId === 'string' ? payload.processId : typeof resultRecord?.processId === 'string' ? resultRecord.processId : undefined : undefined;
  const nativeProcess = resultRecord?.metadata && typeof resultRecord.metadata === 'object'
    ? (resultRecord.metadata as Record<string, unknown>).family === 'harness' : request.connector === 'Harness';
  const savedProcessState = processData?.state ?? payload?.processState ?? payload?.processStatus ?? payload?.status;
  const ownedProcess = useOwnedProcess(message.conversationId, nativeProcess && (savedProcessState === 'running' || !savedProcessState) ? processId : undefined);
  const process = ownedProcess.snapshot ?? processData;
  const processState = processId ? String(process?.state ?? payload?.processState ?? payload?.processStatus ?? payload?.status
    ?? (resultRecord?.isError ? 'failed' : 'exited')) : undefined;
  const frames = Array.isArray(process?.output) ? process.output as { stream?: string; text?: string }[] : [];
  const processStdout = frames.filter(frame => frame.stream !== 'stderr').map(frame => frame.text ?? '').join('');
  const processStderr = frames.filter(frame => frame.stream === 'stderr').map(frame => frame.text ?? '').join('');
  const logsRecord = dataRecord && dataRecord.logs && typeof dataRecord.logs === 'object' && !Array.isArray(dataRecord.logs) ? (dataRecord.logs as Record<string, unknown>) : null;
  // A tool result is either the `locallm.tool-envelope/1` envelope (whose
  // payload lives under `data`) or a legacy bare object. Both shapes are
  // read here so a result is never silently dropped from the transcript.
  const isStructured = (value: Record<string, unknown> | null) =>
    !!value && (typeof value.answer === 'string' || value.sources !== undefined || value.documents !== undefined);
  const checkReport = payload?.report && typeof payload.report === 'object'
    && ['check_run', 'check_report'].includes(name) ? payload.report : undefined;
  const structuredResult = checkReport ?? (dataRecord && 'result' in dataRecord
    ? dataRecord.result
    : isStructured(dataRecord)
    ? dataRecord
    : isStructured(resultRecord)
    ? resultRecord
    : undefined);
  const resultStatus = dataRecord && typeof dataRecord.resultStatus === 'string' ? dataRecord.resultStatus : undefined;
  const singleChange = Array.isArray(payload?.changes) && payload.changes.length === 1 ? payload.changes[0] : null;
  const diff = typeof payload?.diff === 'string' ? payload.diff
    : typeof resultRecord?.diff === 'string' ? resultRecord.diff
    : typeof singleChange?.diff === 'string' ? singleChange.diff : null;
  const stdout = processStdout || (message.status === 'streaming' && output.stdout ? output.stdout : typeof logsRecord?.stdout === 'string' ? logsRecord.stdout : resultRecord && typeof resultRecord.stdout === 'string' ? resultRecord.stdout : typeof dataRecord?.stdout === 'string' ? dataRecord.stdout : '');
  const stderr = processStderr || (message.status === 'streaming' && output.stderr ? output.stderr : typeof logsRecord?.stderr === 'string' ? logsRecord.stderr : resultRecord && typeof resultRecord.stderr === 'string' ? resultRecord.stderr : typeof dataRecord?.stderr === 'string' ? dataRecord.stderr : '');
  const exitCode = typeof process?.exitCode === 'number' ? process.exitCode : dataRecord && typeof dataRecord.exitCode === 'number' ? dataRecord.exitCode : resultRecord && typeof resultRecord.exitCode === 'number' ? resultRecord.exitCode : null;
  const durationMs = typeof process?.durationMs === 'number' ? process.durationMs : typeof dataRecord?.durationMs === 'number' ? dataRecord.durationMs : resultRecord && typeof resultRecord.durationMs === 'number' ? resultRecord.durationMs : undefined;
  const errorRecord = resultRecord && resultRecord.error && typeof resultRecord.error === 'object' && !Array.isArray(resultRecord.error) ? (resultRecord.error as Record<string, unknown>) : null;
  const error = typeof ownedProcess.snapshot?.error === 'string' ? ownedProcess.snapshot.error : typeof errorRecord?.message === 'string' ? errorRecord.message : resultRecord && typeof resultRecord.error === 'string' ? resultRecord.error : null;
  const errorCode = typeof errorRecord?.code === 'string' ? errorRecord.code : undefined;
  const envelopeStatus = resultRecord && typeof resultRecord.status === 'string' ? resultRecord.status : undefined;
  const isError = resultRecord !== null && resultRecord.isError === true;
  const failed = message.status === 'error' || isError || Boolean(error) || (exitCode !== null && exitCode !== 0) || ['error', 'failed', 'failure', 'denied', 'cancelled', 'unknown'].includes(envelopeStatus ?? '');
  const decision = typeof request.decision === 'string' ? request.decision : undefined;
  const status = ownedProcess.notice && processState === 'running' ? 'interrupted' : processState === 'running' ? 'running' : ['stopped', 'unknown'].includes(processState ?? '') ? 'interrupted' : ['failed', 'timed_out'].includes(processState ?? '') ? 'failed' : message.status === 'interrupted'
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

  const diagnosticResult = status === 'failed' && command && !stdout && !stderr && structuredResult == null ? resultRecord : null;
  const hasRecord = Object.keys(record).length > 0;
  const legacyOutput = message.status === 'streaming' ? '' : typeof result === 'string' ? result : hasRecord ? '' : message.content;
  const displayOutput = stdout || legacyOutput;
  // Keep structured records out of the terminal channel so log excerpts show
  // actual output, including for shell tools that return without a log.
  const rawResult = message.status !== 'streaming' && !displayOutput && !stderr && !diff && !diagnosticResult && !processId
    ? resultRecord?.data ?? result ?? (hasRecord ? record : undefined) : undefined;
  const resultValue = structuredResult ?? rawResult;
  const structuredJson = resultValue == null ? null : JSON.stringify(resultValue, null, 2);

  if (/^(workspace_read_file|read_file|view_file|file_read)$/.test(name) && status === 'completed' && typeof resultRecord?._artifactId !== 'string' && typeof payload?.content === 'string') {
    const args = request.arguments as Record<string, unknown> | undefined;
    const totalLines = typeof payload.totalLines === 'number' ? payload.totalLines : undefined;
    const start = args?.start_line ?? (args?.line_count !== undefined ? 1 : undefined);
    const first = payload.content.match(/^(\d+): /)?.[1];
    const last = payload.content.match(/(?:^|\n)(\d+): [^\n]*$/)?.[1];
    const lineRange = typeof start === 'number' && Number(first) === start && last
      ? first === last ? `line ${first}` : `lines ${first}–${last}` : undefined;
    return <article className="message message-tool" aria-label="tool message"><FileResult path={String(payload.path ?? args?.path ?? 'File')} content={payload.content} totalLines={totalLines} lineRange={lineRange} sha256={typeof payload.sha256 === 'string' ? payload.sha256 : undefined} raw={record} /></article>;
  }

  return (
    <article className="message message-tool" aria-label="tool message">
      <ToolResultExtras name={name} result={resultRecord} conversationId={message.conversationId} />
      {ownedProcess.notice && <p className="process-notice" role="status">{ownedProcess.notice}</p>}
      {ownedProcess.snapshot?.truncated && <p className="process-notice">Earlier output was truncated. Showing retained process output.</p>}
      <CommandRunCard
        onStop={processId && nativeProcess && nativeAvailable && !ownedProcess.notice ? () => void ownedProcess.stop() : message.status === 'streaming' ? onStop : undefined}
        stopLabel={processId ? 'Stop process' : 'Stop response'}
        toolName={name}
        command={command}
        code={code}
        language={language}
        cwd={cwd}
        environment={localName ?? displayConnector}
        localServerId={localName ? connector : undefined}
        status={status}
        lifecycleState={processState}
        decision={decision}
        isError={isError}
        exitCode={exitCode}
        stdout={displayOutput}
        stderr={stderr}
        diff={diff}
        durationMs={durationMs}
        error={error}
        errorCode={errorCode}
        resultStatus={resultStatus}
        envelopeStatus={envelopeStatus}
        resultPath={typeof singleChange?.path === 'string' ? singleChange.path : typeof payload?.path === 'string' ? payload.path : undefined}
        structuredResult={structuredJson}
        diagnosticResult={diagnosticResult}
        arguments={request.arguments as Record<string, unknown> | undefined}
        artifactId={artifactId}
        originalBytes={originalBytes}
        authorization={authorization}
      />
      {Array.isArray(resultRecord?._screenshotArtifactIds) && resultRecord._screenshotArtifactIds.filter((id): id is string => typeof id === 'string' && /^art_[a-f0-9]{32}$/.test(id)).slice(0, 2).map(id => <ScreenshotPreview key={id} id={id} conversationId={message.conversationId} />)}
    </article>
  );
});

const MessageBody = memo(function MessageBody({ message, suppressReasoning = false, commentary = false, sourceLinks = '{}', onSendPrompt, onOpenCanvas }: { message: Message; suppressReasoning?: boolean; commentary?: boolean; sourceLinks?: string; onSendPrompt?: (text: string) => void; onOpenCanvas?: (code: string, language: string) => void }) {
  let attached: { name: string; imageUrl?: string; content?: string }[] = [];
  let displayContent = message.content;
  if (message.role === 'user') {
    try { const envelope = JSON.parse(message.content); if (envelope.kind === 'locallm-attachments-v1') { displayContent = envelope.text; attached = envelope.attachments; } } catch { /* Ordinary text message. */ }
  }
  const [copied, setCopied] = useState(false);
  const article = useRef<HTMLElement>(null);
  const [copyError, setCopyError] = useState('');
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  const streaming = message.status === 'streaming';
  if (streaming && !displayContent.trim() && !message.presentation && !message.metrics && !attached.length && (suppressReasoning || !message.reasoning)) return null;

  return (
    <article ref={article} data-message-id={message.id} className={`message message-${message.role}${commentary ? ' message-commentary' : ''}`} aria-label={commentary ? 'Assistant update' : `${message.role} message`}>
      {message.role === 'user' && <span className="transcript-prompt" aria-hidden="true">&gt;</span>}
      <div className="message-byline">{message.role === 'user' ? 'You' : <><span className="mini-mark">L</span>LocalLM</>}</div>
      {commentary && <div className="commentary-label">Update</div>}
      {message.role === 'assistant' && !commentary && <AnswerNavigation article={article} content={streaming ? '' : displayContent} />}
      {message.reasoning && !suppressReasoning && (
        <ReasoningDisclosure text={message.reasoning} active={streaming && !displayContent.trim()} />
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
        <p className={`message-state message-state-${message.status}`} role="status">{message.error || (message.status === 'interrupted' ? 'Response stopped before it finished.' : 'The response stopped because of an error.')}</p>
      )}
      {message.content && !commentary && message.status !== 'streaming' && (
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
          {message.role === 'assistant' && <span>{copied ? 'Copied' : 'Copy answer'}</span>}
        </button>
      )}
      {copyError && <small role="alert">{copyError}</small>}
    </article>
  );
});

const ConversationTurn = memo(function ConversationTurn({ messages, events, run, generating, onSendPrompt, onOpenCanvas, onStop, onResume }: {
  messages: Message[]; events: SessionEvent[]; run?: RunRecord | null; generating: boolean;
  onSendPrompt: (text: string) => void; onOpenCanvas: (code: string, language: string) => void; onStop: () => void;
  onResume?: (taskId?: string) => void;
}) {
  const evidence = useAnswerEvidence(messages, events);
  const sourceLinks = useMemo(() => answerSourceLinks(evidence.sources), [evidence.sources]);
  const research = useMemo(() => {
    const claims: HostClaim[] = events.filter(event => event.eventType === 'verification').flatMap(event => {
      const claims = (event.payload as { claims?: unknown[] })?.claims;
      return Array.isArray(claims) ? claims.slice(0, 100).flatMap(raw => {
        if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return [];
        const claim = raw as { claim?: unknown; status?: unknown; sources?: unknown };
        return typeof claim.claim === 'string' && typeof claim.status === 'string' ? [{ claim: claim.claim.slice(0, 4000), verdict: claim.status,
          citations: Array.isArray(claim.sources) ? claim.sources.filter((id): id is string => typeof id === 'string').flatMap(reference => evidence.sources.find(source => source.url === reference)?.ids ?? [reference]).slice(0, 32) : [] }] : [];
      }) : [];
    });
    return { sources: evidence.sources, claims };
  }, [events, evidence.sources]);
  const renderMessage = useCallback((message: Message, variant?: 'commentary') => <MessageBody message={message} suppressReasoning commentary={variant === 'commentary'} sourceLinks={sourceLinks} onSendPrompt={onSendPrompt} onOpenCanvas={onOpenCanvas} />, [sourceLinks, onSendPrompt, onOpenCanvas]);
  const renderTool = useCallback((message: Message) => <ToolMessage message={message} onStop={onStop} />, [onStop]);
  const user = messages.find(message => message.role === 'user');
  return <div className="conversation-turn">
    {user && <MessageBody message={user} />}
    <ResearchContext.Provider value={research}>
      <WorkSummary messages={messages} events={events} run={run} onResume={onResume} showActivity={!generating}>
        <ActivityTimeline events={events} messages={messages} generating={generating} renderMessage={renderMessage} renderTool={renderTool} />
      </WorkSummary>
    </ResearchContext.Provider>
    <AnswerSources messages={messages} events={events} evidence={evidence} />
    {run && ['failed', 'outcome_unknown', 'paused', 'cancelled'].includes(run.status) && <p role="status" className="run-outcome">Run {run.status.replace(/_/g, ' ')}. Review the activity above before retrying.</p>}
  </div>;
});

export function Chat({
  attachmentDrafts: controlledAttachments,
  onAttachmentDraftsChange,
  composerPermissions,
  reasoningMode,
  conversationKey = 'new',
  draftOwner = conversationKey,
  draftProjectId,
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
  pendingTurn,
  recoveryBusy = false,
  onRecoverPending,
  onDiscardPending,
  onResume,
  onContinueFailed,
  paused = false,
  onConfigure,
  planMode = false,
  onConfigureLocalModel,
  draft: controlledDraft,
  onDraftChange,
  onRestoreDraft,
  liveActivity,
  providers,
  onSelectModel,
  chatError,
  chatNotice,
  onDismissError,
  projectName,
}: Props) {
  const presentation = usePresentationMode(conversationKey);
  const { events: sessionActivity, hasEarlier, loadingEarlier, historyError, loadEarlier } = useSessionActivity(conversationKey, generating);
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
  const latestDraftOwner = useRef(draftOwner);
  latestDraftOwner.current = draftOwner;
  const attachments = attachmentDrafts[draftOwner] ?? EMPTY_ATTACHMENTS;
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
    if (reading.current || generating || loading || disabled || submitting.current.has(draftOwner)) return;
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
      setAttachmentDrafts(current => ({ ...current, [draftOwner]: [...attachments, ...added] }));
    } catch (error) {
      if (latestDraftOwner.current === draftOwner) {
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
  const [contextPreview, setContextPreview] = useState<{ owner: string; breakdown: PreflightBreakdown } | null>(null);
  const preflight = contextPreview?.owner === draftOwner ? contextPreview.breakdown : undefined;
  const [compacting, setCompacting] = useState(false);
  const [compactionNotice, setCompactionNotice] = useState<string | null>(null);
  const [preflightRevision, setPreflightRevision] = useState(0);
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const input = useRef<HTMLTextAreaElement>(null);
  const submitting = useRef(new Set<string>());

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
  const previousTurns = useRef<Message[][]>([]);
  const previousActivity = useRef<SessionEvent[][]>([]);
  const turns = useMemo(() => {
    const grouped: Message[][] = [];
    for (const message of messages) {
      if (message.role === 'user' || !grouped.length) grouped.push([]);
      grouped[grouped.length - 1].push(message);
    }
    const previous = new Map(previousTurns.current.map(turn => [turn[0]?.id, turn]));
    const stable = grouped.map(turn => { const old = previous.get(turn[0]?.id); return old && old.length === turn.length && old.every((message, i) => message === turn[i]) ? old : turn; });
    previousTurns.current = stable;
    return stable;
  }, [messages]);
  const turnActivity = useMemo(() => {
    const next = partitionActivity(turns, sessionActivity).map((events, index) => {
      const old = previousActivity.current[index];
      return old && old.length === events.length && old.every((event, i) => event === events[i]) ? old : events;
    });
    previousActivity.current = next;
    return next;
  }, [turns, sessionActivity]);
  const resumableRuns = useMemo(() => {
    const lastSegments = new Map<string, SessionEvent>();
    for (const event of sessionActivity) {
      if (event.eventType !== 'turn_end') continue;
      const outcome = event.payload as TaskOutcome | null;
      const id = outcome?.taskId ?? outcome?.resumeTaskId;
      if (id) lastSegments.set(id, event);
    }
    return new Set([...lastSegments.values()].filter(event => (event.payload as TaskOutcome).resumeTaskId).map(event => event.runId));
  }, [sessionActivity]);
  const recoverable = lastResponse?.status === 'error' && sessionActivity.some(event => {
    if (event.eventType !== 'turn_end') return false;
    const payload = event.payload as { messageId?: string; recoveryAvailable?: boolean };
    return payload.messageId === lastResponse.id && payload.recoveryAvailable === true;
  });



  // Buttons rendered by generated UI submit a follow-up like a typed message.
  const sendFromUi = useCallback((text: string) => {
    if (!text.trim() || !ready || generating || loading || disabled || submitting.current.has(draftOwner)) return;
    void onSend(text);
  }, [ready, generating, loading, disabled, onSend, draftOwner]);

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
  const transcriptActions = useRef({ sendFromUi, onCancel });
  transcriptActions.current = { sendFromUi, onCancel };
  const sendTranscriptPrompt = useCallback((text: string) => transcriptActions.current.sendFromUi(text), []);
  const stopTranscriptResponse = useCallback(() => transcriptActions.current.onCancel(), []);

  async function retry() {
    if (!lastPrompt || !ready || generating || loading || disabled || submitting.current.has(draftOwner)) return;
    submitting.current.add(draftOwner);
    try {
      await onSend(lastPrompt.content);
    } catch {
      /* App retains and displays the send error. */
    } finally {
      submitting.current.delete(draftOwner);
    }
  }

  useEffect(() => {
    if (follow.current && scroll.current) {
      scroll.current.scrollTop = scroll.current.scrollHeight;
    }
  }, [messages, generating, liveActivity?.activity]);

  useEffect(() => {
    follow.current = true;
    setShowJumpToLatest(false);
  }, [conversationKey]);

  useEffect(() => {
    const content = scroll.current?.querySelector('.messages');
    if (!content || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      if (follow.current && scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight;
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [conversationKey, loading, messages.length > 0]);

  useEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight}px`;
  }, [draft, conversationKey]);

  useEffect(() => {
    let disposed = false;
    setContextPreview(null); setPreflightIssue('');
    if ((!draft.trim() && !attachments.length) || !ready || loading || generating) return;
    let payload: string;
    try { payload = composeMessage(draft, attachments); } catch { return; }
    const timer = setTimeout(() => {
      void api.contextPreflight(conversationKey === 'new' ? null : conversationKey, payload, preset, connectorIds, connectorTools, planMode, reasoningMode, presentation.effectiveMode, draftProjectId).then(result => {
        if (disposed) return;
        if (result.available && result.breakdown) setContextPreview({ owner: draftOwner, breakdown: result.breakdown });
        setPreflightIssue(!result.available && result.reason?.includes('at most 64 tools') ? result.reason : '');
      }).catch(() => { if (!disposed) setContextPreview(null); });
    }, 400);
    return () => { disposed = true; clearTimeout(timer); };
  }, [draft, attachments, ready, loading, generating, conversationKey, draftOwner, draftProjectId, connectorIds, connectorTools, preset, planMode, reasoningMode, modelIdentity, preflightRevision, presentation.effectiveMode]);

  async function submit() {
    if ((!draft.trim() && !attachments.length) || !ready || generating || loading || disabled || preflightIssue || reading.current || submitting.current.has(draftOwner)) return;
    let content: string;
    try {
      content = composeMessage(draft, attachments);
    } catch (error) {
      setAttachmentError((error as Error).message);
      return;
    }
    submitting.current.add(draftOwner);
    const originalDraft = draft;
    setAttachmentError('');
    setAttachmentDrafts(current => ({ ...current, [draftOwner]: [] }));
    setDraft('');
    try {
      await onSend(content);
    } catch (error) {
      const owner = error && typeof error === 'object' && 'conversationId' in error && typeof error.conversationId === 'string' ? error.conversationId : draftOwner;
      if (onRestoreDraft) onRestoreDraft(owner, originalDraft);
      else if (latestDraftOwner.current === draftOwner) setDraft(current => current || originalDraft);
      setAttachmentDrafts(current => ({ ...current, [owner]: attachments }));
    } finally {
      submitting.current.delete(draftOwner);
    }
    if (latestDraftOwner.current === draftOwner) input.current?.focus();
  }

  const isHome = !loading && !messages.length && !generating && !pendingTurn;
  const genericActivity = !liveActivity?.activity || /^(Thinking|Generating response|Streaming response)[….]*$/.test(liveActivity.activity);
  const currentTurn = turns[turns.length - 1] ?? [];
  const streamingAnswer = [...currentTurn].reverse().find(message => message.role === 'assistant' && message.status === 'streaming');
  const answerHasText = Boolean(streamingAnswer?.content.trim());
  const reasoningIsVisible = /^Thinking[….]*$/.test(liveActivity?.activity ?? '') && Boolean(streamingAnswer?.reasoning.trim());
  // Native chat can report the same action through its status and tool stream.
  // Keep the status available until the corresponding transcript row arrives.
  const toolStatus = /^(Executing|Running|Finished) (\S+)$/.exec(liveActivity?.activity ?? '');
  const toolIsActive = toolStatus?.[1] !== 'Finished';
  const toolStatusCovered = toolStatus && !isPlanStateTool(toolStatus[2]) && currentTurn.some(message => {
    if (message.role !== 'tool' || (message.status === 'streaming') !== toolIsActive) return false;
    const record = toolRecord(message);
    return (record.request ?? record).name === toolStatus[2];
  });
  const showLiveStatus = generating && !toolStatusCovered && !(genericActivity && (answerHasText || reasoningIsVisible));
  const liveLabel = genericActivity ? 'Writing the answer' : (liveActivity?.activity || 'Working');
  const currentEvents = turnActivity[turns.length - 1];
  const toolCount = useMemo(() => activityTools(currentEvents ?? [], currentTurn, undefined, generating).length, [currentEvents, currentTurn, generating]);
  const lastAnswer = [...currentTurn].reverse().find(message => message.role === 'assistant');
  const outputTokens = generating ? contextUsage?.outputTokens : sumMetric(lastAnswer?.metrics?.rounds ?? [], 'outputTokens');
  const transcriptPhase = loading ? 'loading' : paused ? 'paused' : generating
    ? reasoningIsVisible ? 'thinking' : toolStatusCovered ? 'running tools' : answerHasText ? 'responding' : 'working'
    : lastAnswer?.status === 'error' ? 'error' : lastAnswer?.status === 'interrupted' ? 'stopped' : lastAnswer?.status === 'complete' ? 'done' : 'idle';

  return (
    <div className="chat-shell" id="active-session-content" role="tabpanel" aria-label="Active conversation">
    <div className={`chat-layout ${isHome ? 'chat-home' : ''}`} style={{ position: 'relative' }}>
      <div className={`chat-content${isHome ? '' : ' chat-transcript'}`}
        onKeyDown={event => {
          if (event.key !== 'Escape' || event.defaultPrevented || !generating || (event.target as Element).closest('dialog, [role="dialog"]')) return;
          event.preventDefault();
          onCancel();
        }}
      >
      <div
        className="chat-scroll"
        ref={scroll}
        role={isHome ? undefined : 'region'}
        aria-label={isHome ? undefined : 'Conversation transcript'}
        tabIndex={isHome ? undefined : 0}
        onClickCapture={event => {
          const target = event.target as HTMLElement;
          const summary = target.closest('summary');
          if (summary && !target.closest('button, a') && !summary.closest('details')?.open) {
            follow.current = false;
            setShowJumpToLatest(true);
          }
        }}
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
        ) : messages.length || generating || pendingTurn ? (
          <div className="messages">
            {hasEarlier && <div className="earlier-activity">
              <button type="button" className="secondary" disabled={loadingEarlier} onClick={loadEarlier}>{loadingEarlier ? 'Loading earlier activity…' : 'Load earlier activity'}</button>
              {historyError && <p role="status">{historyError}</p>}
            </div>}
            {turns.map((turn, turnIndex) => {
              const latest = turnIndex === turns.length - 1;
              const events = turnActivity[turnIndex];
              const canResume = !generating && !pendingTurn && (latest || events.some(event => event.eventType === 'turn_end' && resumableRuns.has(event.runId)));
              return <ConversationTurn key={turn[0].id} messages={turn} events={events} run={latest ? latestRun : undefined} generating={generating && latest} onSendPrompt={sendTranscriptPrompt} onOpenCanvas={openCanvas} onStop={stopTranscriptResponse} onResume={canResume ? onResume : undefined} />;
            })}
            {pendingTurn && !generating && <section className="saved-request" aria-label="Saved unsent request">
              <h2>Unsent request saved</h2>
              <p>This request was saved before sending. Review it, then send it with the current model and tool settings or discard it.</p>
              <details><summary>Review request</summary><MessageBody message={{ id: pendingTurn.id, conversationId: pendingTurn.conversationId, role: 'user', content: pendingTurn.content, reasoning: '', status: 'complete', createdAt: pendingTurn.createdAt }} /></details>
              <div className="saved-request-actions">
                <button type="button" className="secondary" disabled={!ready || loading || recoveryBusy} onClick={onRecoverPending}>Send saved request</button>
                <button type="button" className="secondary" disabled={loading || recoveryBusy} onClick={onDiscardPending}>Discard saved request</button>
              </div>
            </section>}
            {showLiveStatus && (
              <div className="activity-live" role="status">
                <ActivityIndicator active={false} />
                <span className="activity-live-text"><span className={liveActivity?.state === 'awaiting_approval' || liveActivity?.state === 'queued' ? undefined : 'activity-sweep'}>{liveLabel}</span></span>
                {!genericActivity && liveActivity?.state && <span className="activity-badge">{activityStateLabel(liveActivity.state)}</span>}
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
      {!isHome && <footer className="transcript-status" aria-label="Transcript status">
        <span>{transcriptPhase}</span>
        <span>tokens {typeof outputTokens === 'number' && Number.isFinite(outputTokens) ? outputTokens.toLocaleString() : '—'}</span>
        <span>tools {toolCount}</span>
        <div className="transcript-actions">
          {showJumpToLatest && <button type="button" onClick={() => {
            if (!scroll.current) return;
            scroll.current.scrollTo({ top: scroll.current.scrollHeight, behavior: prefersReducedMotion() ? 'instant' : 'smooth' });
            follow.current = true;
            setShowJumpToLatest(false);
          }} aria-label="Jump to latest message"><ArrowDown size={12} aria-hidden="true" /> latest</button>}
          {generating && <button type="button" aria-keyshortcuts="Escape" aria-label="Interrupt response" onClick={onCancel}>esc to interrupt</button>}
        </div>
      </footer>}
      </div>

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
            {recoverable && onContinueFailed && <button disabled={!ready || loading || disabled} onClick={() => onContinueFailed(lastResponse.id)}>Continue from saved results</button>}
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
            {planMode && <span className="composer-plan-label">Plan before acting</span>}
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
                      <small>{Math.max(1, Math.ceil(file.size / 1024))} KB · {file.imageUrl ? 'Image' : file.mediaUrl?.startsWith('data:video/') ? 'Video' : file.mediaUrl ? 'PDF' : 'Text'}</small>
                    </span>
                    <button
                      type="button"
                      className="icon-button"
                      aria-label={`Remove ${file.name}`}
                      disabled={readingFiles}
                      onClick={() =>
                        setAttachmentDrafts(current => ({
                          ...current,
                          [draftOwner]: attachments.filter(item => item.id !== file.id),
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
              const files = Array.from(event.clipboardData?.files ?? []);
              if (!files.length) {
                for (const item of Array.from(event.clipboardData?.items ?? [])) {
                  if (item.kind !== 'file') continue;
                  const file = item.getAsFile();
                  if (file) files.push(file);
                }
              }
              if (!files.length) return;
              event.preventDefault();
              void attach(files);
            }}
          />
          <div className="composer-bottom">
            <div className="composer-actions">
              <button
                type="button"
                className="icon-button attach-button"
                aria-label="Add files"
                title="Attach images, PDFs, MP4 videos, or text files"
                disabled={readingFiles || generating || loading || disabled}
                onClick={() => fileInput.current?.click()}
              >
                <Plus size={20} />
              </button>
              {composerPermissions}
            </div>
            <div className="composer-send" style={{ position: 'relative' }}>
              {(!isHome || ready || contextUsage) && <ContextControl
                conversationId={conversationKey}
                busy={generating || loading}
                usage={contextUsage}
                preview={preflight}
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

              {paused && !generating && onResume && !pendingTurn && !turnActivity[turnActivity.length - 1]?.some(event => event.eventType === 'turn_end') && <button type="button" className="send-button" aria-label="Resume response" onClick={() => onResume()}><Play size={15} fill="currentColor" /></button>}
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
        {preflight?.presentation && <details className="composer-note"><summary>Visual response budget</summary><p>Separate formatting request: at least {preflight.presentation.minimumInputTokens.toLocaleString()} input tokens plus the original answer, with {preflight.presentation.responseReserve.toLocaleString()} output tokens reserved. The full request is checked before generation; text remains available if it does not fit.</p></details>}
        {preflight?.overflow && (
          <p className="composer-note" role="alert">
            <span className="context-overflow">{preflight.overflow}</span>
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
