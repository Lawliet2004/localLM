import { type Dispatch, type SetStateAction, type ReactNode, useEffect, useRef, useState } from 'react';
import { ArrowDown, ArrowUp, Check, ChevronDown, Copy, Cpu, FileText, MessageSquare, Plus, Square, Terminal, WandSparkles, X } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { attachmentAccept, composeMessage, readAttachment, type Attachment } from '../lib/attachments';
import { api, errorMessage } from '../lib/api';
import type { Message, ContextUsage, ModelSelection, ProviderConnection } from '../lib/types';

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
  onSend: (content: string) => Promise<void>;
  onCancel: () => void;
  onConfigure: () => void;
  liveActivity?: { state?: string; activity?: string } | null;
  providers?: ProviderConnection[];
  onSelectModel?: (selection: ModelSelection) => Promise<void>;
  chatError?: string;
  onDismissError?: () => void;
}

function ToolMessage({ message }: { message: Message }) {
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
  const diff = resultRecord && typeof resultRecord.diff === 'string' ? (resultRecord.diff as string) : null;
  const displayResult = resultRecord ? Object.fromEntries(Object.entries(resultRecord).filter(([key]) => key !== 'diff')) : (result ?? message.content);
  const failed = message.status === 'error' || (resultRecord !== null && resultRecord.isError === true);
  const status = message.status === 'interrupted' ? 'Stopped · outcome unknown' : request.decision === 'denied' ? 'Denied' : message.status === 'streaming' ? 'Running…' : failed ? 'Failed' : 'Finished';

  const artifactId = resultRecord && typeof resultRecord._artifactId === 'string' ? resultRecord._artifactId : null;
  const [artifactContent, setArtifactContent] = useState<string | null>(null);
  const [loadingArtifact, setLoadingArtifact] = useState(false);

  async function inspectArtifact() {
    if (!artifactId || artifactContent !== null || loadingArtifact) return;
    setLoadingArtifact(true);
    try {
      const art = await api.getArtifact(artifactId);
      setArtifactContent(art ? art.content : 'Artifact not found.');
    } catch (e) {
      setArtifactContent(`Could not load artifact: ${errorMessage(e)}`);
    } finally {
      setLoadingArtifact(false);
    }
  }

  return (
    <article className="message message-tool" aria-label="tool message">
      <details className="tool-record">
        <summary>
          <Terminal size={14} />
          <strong>{localName ?? connector} · {name}</strong>
          <span>{status}</span>
        </summary>
        {localName && <p>Local server ID: <code>{connector}</code></p>}
        {typeof request.authorization === 'string' && <p>Authorization: {request.authorization}</p>}
        {request.connector === 'Workspace' && request.name === 'edit_file' && (
          <p>Review the unified diff below before continuing. Re-read the file when the hash no longer matches.</p>
        )}
        {artifactId && (
          <div className="tool-artifact-box">
            <div className="tool-artifact-header">
              <span>Full result captured in artifact: <code>{artifactId}</code></span>
              {typeof resultRecord?._originalBytes === 'number' && (
                <small>({(resultRecord._originalBytes / 1024).toFixed(1)} KB)</small>
              )}
              <button
                type="button"
                className="secondary"
                style={{ fontSize: '11px', padding: '2px 8px', marginLeft: 'auto' }}
                disabled={loadingArtifact}
                onClick={() => void inspectArtifact()}
              >
                {loadingArtifact ? 'Loading…' : artifactContent !== null ? 'Loaded' : 'Inspect artifact'}
              </button>
            </div>
            {artifactContent !== null && (
              <pre className="tool-artifact-content">{artifactContent}</pre>
            )}
          </div>
        )}
        <h4>Arguments</h4>
        <pre>{JSON.stringify(request.arguments ?? {}, null, 2)}</pre>
        <h4>Result</h4>
        <pre>{JSON.stringify(displayResult, null, 2)}</pre>
        {diff && (
          <>
            <h4>Diff</h4>
            <pre className="tool-diff">{diff}</pre>
          </>
        )}
      </details>
    </article>
  );
}

function MessageBody({ message }: { message: Message }) {
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState('');
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return (
    <article className={`message message-${message.role}`} aria-label={`${message.role} message`}>
      <div className="message-byline">{message.role === 'user' ? 'You' : <><span className="mini-mark">L</span>LocalLM</>}</div>
      {message.reasoning && (
        <details className="reasoning">
          <summary>{message.status === 'streaming' ? 'Thinking…' : 'Reasoning'}</summary>
          <div>{message.reasoning}</div>
        </details>
      )}
      <div className="markdown">
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          components={{
            a: ({ children, href }) => <a href={href} target="_blank" rel="noreferrer">{children}</a>,
            img: ({ alt }) => <span className="image-reference">Image: {alt || 'external image'}</span>,
          }}
        >
          {message.content}
        </ReactMarkdown>
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
  onSend,
  onCancel,
  onConfigure,
  draft: controlledDraft,
  onDraftChange,
  liveActivity,
  providers,
  onSelectModel,
  chatError,
  onDismissError,
}: Props) {
  const [localAttachments, setLocalAttachments] = useState<Record<string, Attachment[]>>({});
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
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const input = useRef<HTMLTextAreaElement>(null);
  const submitting = useRef(false);

  const lastPrompt = [...messages].reverse().find(message => message.role === 'user');
  const lastResponse = [...messages].reverse().find(message => message.role === 'assistant');
  const canRetry = lastPrompt && lastResponse && ['error', 'interrupted'].includes(lastResponse.status);

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
    if (input.current) {
      input.current.style.height = 'auto';
      input.current.style.height = `${Math.min(input.current.scrollHeight, 200)}px`;
    }
  }, [draft]);

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
            {messages.map(message =>
              message.role === 'tool' ? (
                <ToolMessage key={message.id} message={message} />
              ) : (
                <MessageBody key={message.id} message={message} />
              )
            )}
            {generating && (
              <div className="activity-timeline" role="status">
                <span className="activity-dot" />
                <span>{liveActivity?.activity || 'Generating response…'}</span>
                {liveActivity?.state && <span className="activity-badge">{liveActivity.state}</span>}
              </div>
            )}
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
          </div>
        ) : (
          <div className="welcome">
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
        <form
          className={`composer ${dragging ? 'composer-dragging' : ''}`}
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
                  <span>
                    <strong>{file.name}</strong>
                    <small>{Math.max(1, Math.ceil(file.size / 1024))} KB · Text</small>
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
          {readingFiles && <p role="status">Reading files…</p>}
          {dragging && <p className="attachment-drop-hint">Drop text, code, or CSV files here</p>}
          <textarea
            ref={input}
            aria-label="Message"
            placeholder="Ask anything, or work on an idea…"
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
          />
          <div className="composer-bottom">
            <div className="composer-actions">
              <button
                type="button"
                className="icon-button attach-button"
                aria-label="Add files"
                title="Attach text, code, or CSV files"
                disabled={readingFiles || generating || loading || disabled}
                onClick={() => fileInput.current?.click()}
              >
                <Plus size={20} />
              </button>
              <div className="composer-tools">{composerTools}</div>
            </div>
            <div className="composer-send" style={{ position: 'relative' }}>
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

              {generating ? (
                <button type="button" className="send-button" aria-label="Stop response" onClick={onCancel}>
                  <Square size={15} fill="currentColor" />
                </button>
              ) : (
                <button
                  className="send-button"
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
        <p className="composer-note">Shift + Enter for a new line · Attach text, code, or CSV files</p>
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
