import { type Dispatch, type SetStateAction, type ReactNode, useEffect, useRef, useState } from 'react';
import { ArrowDown, ArrowUp, Check, ChevronDown, ChevronRight, Copy, Cpu, FileText, Folder, GitBranch, Laptop, ListTodo, MessageSquare, Mic, Plus, Square, Terminal, WandSparkles, X } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { attachmentAccept, composeMessage, readAttachment, type Attachment } from '../lib/attachments';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { modelLabel as localModelLabel } from '../lib/localModels';
import type { Message, ContextUsage, ModelSelection, ProviderConnection, PreflightBreakdown, ToolSelection } from '../lib/types';
import type { InstalledModel } from '../lib/types';
import { WorkSummary } from './WorkSummary';
import { ActivityTimeline, useSessionActivity } from './ActivityTimeline';
import { ContextControl } from './ContextControl';
import { CommandRunCard } from './CommandRunCard';
import type { RunRecord } from '../lib/types';

interface Props {
  composerTools?: ReactNode;
  conversationKey?: string;
  attachmentDrafts?: Record<string, Attachment[]>;
  onAttachmentDraftsChange?: Dispatch<SetStateAction<Record<string, Attachment[]>>>;
  contextUsage?: ContextUsage;
  modelLabel?: string;
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
  onConfigure: () => void;
  planMode?: boolean;
  onPlanModeChange?: (next: boolean) => void;
  onConfigureLocalModel?: (filename: string) => void;
  liveActivity?: { state?: string; activity?: string } | null;
  providers?: ProviderConnection[];
  onSelectModel?: (selection: ModelSelection) => Promise<void>;
  chatError?: string;
  chatNotice?: string;
  onDismissError?: () => void;
  projectName?: string;
  gitBranch?: string;
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
  const structuredResult = dataRecord && 'result' in dataRecord ? dataRecord.result : undefined;
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

  const fallbackStdout = stdout || (!resultRecord && typeof result === 'string' ? result : (resultRecord ? JSON.stringify(resultRecord, null, 2) : message.content));
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

function TurnTools({ tools, onCancel }: { tools: Message[]; onCancel: () => void }) {
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({});
  if (!tools.length) return null;

  // Segment tools into edits vs non-edits
  const segments: { type: 'edit' | 'group'; tools: Message[]; title?: string; key: string }[] = [];
  let currentGroup: Message[] = [];

  const flushGroup = () => {
    if (!currentGroup.length) return;
    const groupTools = [...currentGroup];
    let fileCount = 0;
    let searchCount = 0;
    let folderCount = 0;
    let taskCount = 0;
    let commandCount = 0;
    const isStreaming = groupTools.some(t => t.status === 'streaming');

    for (const msg of groupTools) {
      try {
        const val = JSON.parse(msg.content);
        const req = val.request || val;
        const name = req.name || '';
        const args = req.arguments || {};
        if (/search|grep|find|locate/.test(name)) searchCount++;
        else if (/list_dir|list_files|browse_dir/.test(name)) folderCount++;
        else if (name === 'manage_task') taskCount++;
        else if (args.command || args.CommandLine || /exec|run_code|terminal|run_command/.test(name)) commandCount++;
        else fileCount++;
      } catch {
        fileCount++;
      }
    }

    let title = '';
    if (commandCount > 0 && fileCount === 0 && searchCount === 0 && folderCount === 0 && taskCount === 0) {
      const prefix = isStreaming ? 'Running' : 'Ran';
      title = `${prefix} ${commandCount} ${commandCount === 1 ? 'command' : 'commands'}`;
    } else {
      const parts: string[] = [];
      if (fileCount > 0) parts.push(`${fileCount} ${fileCount === 1 ? 'file' : 'files'}`);
      if (folderCount > 0) parts.push(`${folderCount} ${folderCount === 1 ? 'folder' : 'folders'}`);
      if (searchCount > 0) parts.push(`${searchCount} ${searchCount === 1 ? 'search' : 'searches'}`);
      if (taskCount > 0) parts.push(`${taskCount} ${taskCount === 1 ? 'task' : 'tasks'}`);
      if (!parts.length && commandCount > 0) parts.push(`${commandCount} ${commandCount === 1 ? 'command' : 'commands'}`);
      if (!parts.length) parts.push(`${groupTools.length} ${groupTools.length === 1 ? 'item' : 'items'}`);
      const prefix = isStreaming ? 'Exploring' : 'Explored';
      title = `${prefix} ${parts.join(', ')}`;
    }

    segments.push({
      type: 'group',
      tools: groupTools,
      title,
      key: `grp-${groupTools[0].id}`,
    });
    currentGroup = [];
  };

  for (const tool of tools) {
    let isEdit = false;
    try {
      const val = JSON.parse(tool.content);
      const req = val.request || val;
      const res = val.result || {};
      const name = req.name || '';
      isEdit = /edit|create_file|write_to_file|replace/.test(name) || Boolean(res.diff);
    } catch { /* ignore */ }

    if (isEdit) {
      flushGroup();
      segments.push({ type: 'edit', tools: [tool], key: `edit-${tool.id}` });
    } else {
      currentGroup.push(tool);
    }
  }
  flushGroup();

  return (
    <div className="activity-feed">
      {segments.map((seg, segIdx) => {
        if (seg.type === 'edit') {
          return <ToolMessage key={seg.key} message={seg.tools[0]} onStop={onCancel} />;
        }
        const isLast = segIdx === segments.length - 1;
        const isStreaming = seg.tools.some(t => t.status === 'streaming');
        const defaultExpanded = isStreaming || isLast || seg.tools.length <= 1;
        const isExpanded = openGroups[seg.key] ?? defaultExpanded;
        return (
          <div key={seg.key} className="activity-group">
            <button
              type="button"
              className={`activity-group-header ${isExpanded ? 'expanded' : ''}`}
              onClick={() => setOpenGroups(curr => ({ ...curr, [seg.key]: !isExpanded }))}
              aria-expanded={isExpanded}
            >
              <span>{seg.title}</span>
              {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
            </button>
            <div className="activity-group-items" style={{ display: isExpanded ? 'flex' : 'none' }}>
              {seg.tools.map(tool => (
                <ToolMessage key={tool.id} message={tool} onStop={onCancel} />
              ))}
            </div>
          </div>
        );
      })}
    </div>
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
  if (!src) return <span className="image-reference">Image: {alt || 'image'}</span>;
  return (
    <>
      <span className="image-preview-wrapper" onClick={() => setIsOpen(true)} title="Click to enlarge">
        <img src={src} alt={alt || 'Image preview'} className="image-preview-thumb" />
      </span>
      {isOpen && (
        <div className="image-lightbox-overlay" onClick={() => setIsOpen(false)}>
          <div className="image-lightbox-modal" onClick={e => e.stopPropagation()}>
            <img src={src} alt={alt || 'Enlarged image'} className="lightbox-img" />
            <button
              type="button"
              className="lightbox-close-btn"
              onClick={() => setIsOpen(false)}
              aria-label="Close preview"
            >
              <X size={18} />
            </button>
          </div>
        </div>
      )}
    </>
  );
}

function MessageBody({ message }: { message: Message }) {
  let attached: { name: string; imageUrl?: string; content?: string }[] = [];
  let displayContent = message.content;
  if (message.role === 'user') {
    try { const envelope = JSON.parse(message.content); if (envelope.kind === 'locallm-attachments-v1') { displayContent = envelope.text; attached = envelope.attachments; } } catch { /* Ordinary text message. */ }
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

  useEffect(() => () => clearTimeout(timer.current), []);

  useEffect(() => {
    if (message.status === 'streaming') {
      isStreamingInitially.current = true;
      setIsOpen(true);
      userToggled.current = false;
      const interval = setInterval(() => {
        const secs = ((Date.now() - startTime.current) / 1000).toFixed(1);
        setElapsed(secs);
      }, 100);
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
  }, [message.status]);

  const thoughtSummary = message.status === 'streaming'
    ? `Thinking (${elapsed}s)…`
    : finalDuration
    ? `Thought for ${Math.round(parseFloat(finalDuration)) || finalDuration}s`
    : 'Thought';

  return (
    <article data-message-id={message.id} className={`message message-${message.role}`} aria-label={`${message.role} message`}>
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
            {message.status === 'streaming' && <span className="thinking-pulsar" />}
            <span>{thoughtSummary}</span>
            {isOpen ? <ChevronDown size={13} className="thought-chevron" /> : <ChevronRight size={13} className="thought-chevron" />}
          </summary>
          <div className="reasoning-content trace-thought-content">{message.reasoning}</div>
        </details>
      )}
      <div className="markdown">
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
}

export function Chat({
  attachmentDrafts: controlledAttachments,
  onAttachmentDraftsChange,
  composerTools,
  conversationKey = 'new',
  contextUsage,
  modelLabel = 'Local model',
  selectionIssue = false,
  messages,
  generating,
  ready,
  loading,
  disabled = false,
  preset = 'standard',
  connectorIds = [],
  connectorTools = [],
  onSend,
  onCancel,
  onConfigure,
  planMode = false,
  onPlanModeChange,
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
  gitBranch,
}: Props) {
  const sessionActivity = useSessionActivity(conversationKey, generating);
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
  const attachments = attachmentDrafts[conversationKey] ?? [];
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
      if (draft.trim()) {
        void api.contextPreflight(conversationKey, draft, preset, connectorIds, connectorTools, planMode).then(result => {
          if (result.available) setPreflight({ breakdown: result.breakdown!, draft });
        });
      }
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
  const turns: Message[][] = [];
  for (const message of messages) {
    if (message.role === 'user' || !turns.length) turns.push([]);
    turns[turns.length - 1].push(message);
  }


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
  }, [messages]);

  useEffect(() => {
    if (preflightTimer.current) clearTimeout(preflightTimer.current);
    if (!draft.trim() || !ready) { setPreflight(null); return; }
    preflightTimer.current = setTimeout(() => {
      void api.contextPreflight(conversationKey === 'new' ? null : conversationKey, draft, preset, connectorIds, connectorTools, planMode).then(result => {
        if (result.available) setPreflight({ breakdown: result.breakdown!, draft });
        else setPreflight(null);
      }).catch(() => setPreflight(null));
    }, 400);
    return () => { if (preflightTimer.current) clearTimeout(preflightTimer.current); };
  }, [draft, ready, conversationKey, connectorIds, connectorTools, preset, planMode]);

  async function submit() {
    if ((!draft.trim() && !attachments.length) || !ready || generating || loading || disabled || reading.current || submitting.current) return;
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
        ) : messages.length ? (
          <div className="messages">
            {turns.map((turn, turnIndex) => {
              const assistants = turn.filter(message => message.role === 'assistant');
              const user = turn.find(message => message.role === 'user');
              const recordedRun = user?.id === lastPrompt?.id && latestRun?.conversationId === conversationKey && latestRun.createdAt >= (user?.createdAt ?? 0) ? latestRun : null;
              const activity = sessionActivity.filter(e => e.createdAt >= (user?.createdAt ?? 0) && e.createdAt < (turns[turnIndex + 1]?.[0].createdAt ?? Infinity));
              if (activity.length > 0) return <div key={turn[0].id} className="conversation-turn">
                {user && <MessageBody message={user} />}
                <ActivityTimeline events={activity} messages={turn} renderMessage={m => <MessageBody message={m} />} renderTool={m => <ToolMessage message={m} onStop={onCancel} />} />
              </div>;
              return <div key={turn[0].id} className="conversation-turn">
                {user && <MessageBody message={user} />}
                <TurnTools tools={turn.filter(message => message.role === 'tool')} onCancel={onCancel} />
                {assistants.map((message, index) => index === assistants.length - 1 && message.status !== 'streaming'
                  ? <WorkSummary key={message.id} messages={turn} run={recordedRun}><MessageBody message={message} /></WorkSummary>
                  : <MessageBody key={message.id} message={message} />)}
              </div>;
            })}
          </div>
        ) : (
          <div className="welcome codex-welcome">
            <div className="codex-cloud-icon-wrapper">
              <svg width="56" height="50" viewBox="0 0 64 60" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" className="codex-cloud-icon">
                <path d="M 21 44 C 15 44 11 39 11 34 C 11 29 15 25 20 24 C 20 17 26 12 34 12 C 42 12 48 17 48 23 C 53 25 56 29 56 34 C 56 39 52 44 46 44 C 43 48 39 50 34 50 C 28 50 24 47 21 44 Z" />
                <path d="M 26 28 L 31 32 L 26 36" />
                <line x1="34" y1="36" x2="40" y2="36" />
              </svg>
            </div>
            <h1 className="codex-welcome-heading">
              {projectName ? <>What should we build in <u className="project-highlight">{projectName}</u>?</> : 'What should we work on?'}
            </h1>
            <div className="sr-only">
              <div className="welcome-mark"><span /> <span /> <span /></div>
              <p className="eyebrow">YOUR LOCAL WORKSPACE</p>
              <h1>A little model.<br />Room for big ideas.</h1>
              <p className="welcome-description">
                Think out loud, work through a problem, or start something new.<br className="wide-only" /> Your model runs right here, on your computer.
              </p>
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
                    <Icon size={18} />
                    <span>{title}</span>
                    <span className="suggestion-arrow">↗</span>
                  </button>
                ))}
              </div>
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
        {generating && (
          <div className="activity-timeline" role="status">
            <span className="activity-dot" />
            <span>{liveActivity?.activity || 'Generating response…'}</span>
            {liveActivity?.state && <span className="activity-badge">{liveActivity.state}</span>}
          </div>
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
            <span>{selectionIssue ? 'Choose a tested provider and configure its model limits, or select the local model.' : 'Load a local model or choose a tested API provider to start a conversation.'}</span>
            <button onClick={onConfigure}>Open Models <span>↗</span></button>
          </div>
        )}
        <div className="codex-composer-wrapper">
          <div className="composer-meta-bar">
            {projectName ? <><div className="composer-meta-item">
              <Folder size={13} className="composer-meta-icon" />
              <span>{projectName}</span>
            </div><div className="composer-meta-item">
              <Laptop size={13} className="composer-meta-icon" />
              <span>Local</span>
            </div>{gitBranch && <div className="composer-meta-item">
              <GitBranch size={13} className="composer-meta-icon" />
              <span>{gitBranch}</span>
            </div>}</> : <div className="composer-meta-item">
              <Folder size={13} className="composer-meta-icon" />
              <span>No workspace</span>
            </div>}
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
            placeholder="Do anything"
            value={draft}
            rows={2}
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
              <button
                type="button"
                className={`icon-button plan-toggle ${planMode ? 'active' : ''}`}
                aria-pressed={planMode}
                aria-label="Plan mode"
                title={planMode ? 'Plan mode is on: write a step-by-step plan, then implement it with tools' : 'Plan mode: for complex tasks, create a plan first, then implement it step by step'}
                disabled={generating || loading || disabled}
                onClick={() => onPlanModeChange?.(!planMode)}
              >
                <ListTodo size={16} />
                {planMode && <span className="plan-toggle-label">Plan</span>}
              </button>
              <div className="composer-tools">{composerTools}</div>
            </div>
            <div className="composer-send" style={{ position: 'relative' }}>
              <ContextControl conversationId={conversationKey} busy={generating || loading} usage={contextUsage} />
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
                    {onConfigureLocalModel && downloadedModels.filter(model => model.complete).map(model => <button type="button" className="composer-model-option" key={model.id} onClick={() => { setShowModelPicker(false); onConfigureLocalModel(model.id); }}><strong>{localModelLabel(model.filename)}</strong><small>{model.repo ?? "Local GGUF"} · Use model</small></button>)}
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
                    {providers?.filter(p => p.verified).map(p => (
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
                      Configure in Models & runtime ↗
                    </button>
                  </div>
                </div>
              )}

              <button
                type="button"
                className="icon-button composer-mic-btn"
                aria-label="Voice input"
                title="Voice input"
              >
                <Mic size={16} />
              </button>

              {(!draft.trim() && !attachments.length && !generating) && (
                <button
                  type="button"
                  className="composer-voice-orb"
                  aria-label="Voice mode"
                  title="Voice mode"
                >
                  <span className="wave-bar bar-1" />
                  <span className="wave-bar bar-2" />
                  <span className="wave-bar bar-3" />
                  <span className="wave-bar bar-4" />
                </button>
              )}

              {generating ? (
                <button type="button" className="send-button" aria-label="Stop response" onClick={onCancel}>
                  <Square size={15} fill="currentColor" />
                </button>
              ) : (
                <button
                  className={`send-button ${(!draft.trim() && !attachments.length) ? 'sr-only' : ''}`}
                  type="submit"
                  aria-label="Send message"
                  disabled={(!draft.trim() && !attachments.length) || !ready || loading || disabled || readingFiles}
                >
                  <ArrowUp size={20} />
                </button>
              )}
            </div>
          </div>
        </form>
        </div>
        <p className={`composer-note ${!messages.length ? 'sr-only' : ''}`}>Shift + Enter for a new line · Attach images, text, code, or CSV files</p>
        {compactionNotice && (
          <p className="composer-note compaction-notice" role="status">
            {compactionNotice}
          </p>
        )}
        {preflight && (
          <p className="composer-note" aria-label="Context preview">
            {preflight.breakdown.exact ? 'Exact' : 'Estimated'} tokens: {preflight.breakdown.total.toLocaleString()} input ({preflight.breakdown.instructions.toLocaleString()} instructions, {preflight.breakdown.tools.toLocaleString()} tools, {preflight.breakdown.history.toLocaleString()} history{preflight.breakdown.scratchpad ? `, ${preflight.breakdown.scratchpad.toLocaleString()} scratchpad` : ''}, {preflight.breakdown.draft.toLocaleString()} draft) + {preflight.breakdown.responseReserve.toLocaleString()} reserve / {preflight.breakdown.contextLength.toLocaleString()} context.
            {preflight.breakdown.overflow && <span className="context-overflow"> {preflight.breakdown.overflow}</span>}
            {conversationKey !== 'new' && messages.length > 2 && (
              <button
                type="button"
                className="compact-button secondary inline-compact-btn"
                disabled={compacting || loading || generating}
                onClick={() => void handleCompact()}
                title="Compact older conversation history into an artifact"
              >
                {compacting ? 'Compacting…' : 'Compact history'}
              </button>
            )}
          </p>
        )}
        {contextUsage && (
          <p className="composer-note" aria-label="Last request context">
            Last request: {contextUsage.estimated ? 'estimated ' : ''}
            {contextUsage.inputTokens.toLocaleString()} input + {contextUsage.responseReserve.toLocaleString()} response reserve / {contextUsage.contextLength.toLocaleString()} context tokens. Draft changes are not included.
          </p>
        )}
      </div>
    </div>
  );
}
