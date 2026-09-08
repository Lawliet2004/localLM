import { useEffect, useRef, useState } from 'react';
import { ArrowUp, Check, Copy, Cpu, MessageSquare, Square, Terminal, WandSparkles } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import type { Message, ContextUsage } from '../lib/types';

interface Props {
  contextUsage?: ContextUsage;
  draft?: string; onDraftChange?: (value: string | ((previous: string) => string)) => void;
  messages: Message[]; generating: boolean; ready: boolean; loading: boolean;
  disabled?: boolean;
  onSend: (content: string) => Promise<void>; onCancel: () => void; onConfigure: () => void;
}
function ToolMessage({ message }: { message: Message }) {
  let record: Record<string, unknown> = {};
  try { const value: unknown = JSON.parse(message.content); if (value && typeof value === 'object' && !Array.isArray(value)) record = value as Record<string, unknown>; } catch { /* Preserve legacy tool content below. */ }
  const request = record.request && typeof record.request === 'object' ? record.request as Record<string, unknown> : record;
  const name = typeof request.name === 'string' ? request.name : 'Tool';
  const connector = typeof request.connector === 'string' ? request.connector : 'Connector';
  const localName = typeof request.localServerName === 'string' && request.localServerName ? request.localServerName : undefined;
  const result = record.result;
  const failed = message.status === 'error' || (result !== null && typeof result === 'object' && 'isError' in result && result.isError === true);
  const status = message.status === 'interrupted' ? 'Stopped · outcome unknown' : request.decision === 'denied' ? 'Denied' : message.status === 'streaming' ? 'Running…' : failed ? 'Failed' : 'Finished';
  return <article className="message message-tool" aria-label="tool message"><details className="tool-record"><summary><Terminal size={14} /><strong>{localName ?? connector} · {name}</strong><span>{status}</span></summary>{localName && <p>Local server ID: <code>{connector}</code></p>}{typeof request.authorization === 'string' && <p>Authorization: {request.authorization}</p>}<h4>Arguments</h4><pre>{JSON.stringify(request.arguments ?? {}, null, 2)}</pre><h4>Result</h4><pre>{JSON.stringify(record.result ?? message.content, null, 2)}</pre></details></article>;
}
function MessageBody({ message }: { message: Message }) {
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState('');
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return <article className={`message message-${message.role}`} aria-label={`${message.role} message`}>
    <div className="message-byline">{message.role === 'user' ? 'You' : <><span className="mini-mark">L</span>LocalLM</>}</div>
    {message.reasoning && <details className="reasoning"><summary>{message.status === 'streaming' ? 'Thinking…' : 'Reasoning'}</summary><div>{message.reasoning}</div></details>}
    <div className="markdown"><ReactMarkdown remarkPlugins={[remarkGfm]} components={{
      a: ({ children, href }) => <a href={href} target="_blank" rel="noreferrer">{children}</a>,
      img: ({ alt }) => <span className="image-reference">Image: {alt || 'external image'}</span>,
    }}>{message.content}</ReactMarkdown></div>
    {message.status === 'streaming' && !message.content && !message.reasoning && <span className="thinking" role="status">Thinking<span>···</span></span>}
    {(message.status === 'interrupted' || message.status === 'error') && <p className="message-state">{message.error || (message.status === 'interrupted' ? 'Response stopped' : 'Response interrupted by an error')}</p>}
    {message.content && <button className="icon-button copy-message" aria-label="Copy message" title="Copy message" onClick={async () => {
      try { await navigator.clipboard.writeText(message.content); setCopied(true); clearTimeout(timer.current); timer.current = setTimeout(() => setCopied(false), 2000); }
      catch { setCopyError('Could not access the clipboard. Select the text to copy it.'); }
    }}>{copied ? <Check size={14} /> : <Copy size={14} />}</button>}
    {copyError && <small role="alert">{copyError}</small>}
  </article>;
}

export function Chat({ contextUsage, messages, generating, ready, loading, disabled = false, onSend, onCancel, onConfigure, draft: controlledDraft, onDraftChange }: Props) {
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
    try { await onSend(lastPrompt.content); } catch { /* App retains and displays the send error. */ }
    finally { submitting.current = false; }
  }
  useEffect(() => { if (follow.current && scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight; }, [messages]);
  async function submit() {
    if (!draft.trim() || !ready || generating || loading || disabled || submitting.current) return;
    submitting.current = true;
    const content = draft;
    setDraft('');
    try { await onSend(content); } catch { setDraft(current => current || content); }
    finally { submitting.current = false; }
    input.current?.focus();
  }
  return <div className="chat-layout">
    <div className="chat-scroll" ref={scroll} onScroll={() => {
      const element = scroll.current;
      if (element) follow.current = element.scrollHeight - element.scrollTop - element.clientHeight < 120;
    }}>
      {loading ? <p className="loading-state" role="status">Opening conversation…</p> : messages.length ? <div className="messages">{messages.map(message => message.role === 'tool' ? <ToolMessage key={message.id} message={message} /> : <MessageBody key={message.id} message={message} />)}</div> :
        <div className="welcome"><div className="welcome-mark"><span /> <span /> <span /></div><p className="eyebrow">YOUR LOCAL WORKSPACE</p><h1>A little model.<br />Room for big ideas.</h1><p className="welcome-description">Think out loud, work through a problem, or start something new.<br className="wide-only" /> Your model runs right here, on your computer.</p>
          <div className="suggestions">{[
            { icon: MessageSquare, title: 'Think it through', prompt: 'Help me think through an idea. Start by asking me what I want to achieve.' },
            { icon: Terminal, title: 'Write some code', prompt: 'Help me write a small, useful program. Ask me what it should do.' },
            { icon: WandSparkles, title: 'Make a plan', prompt: 'Help me turn a goal into a practical plan. Ask me about my goal first.' },
          ].map(({ icon: Icon, title, prompt }) => <button key={title} onClick={() => { setDraft(prompt); input.current?.focus(); }}><Icon size={18} /><span>{title}</span><span className="suggestion-arrow">↗</span></button>)}</div>
        </div>}
    </div>
    <div className="composer-region">
      {canRetry && !generating && <div className="setup-hint"><span>Retry sends the last prompt as a new message. Enabled tool actions may run again.</span><button disabled={!ready || loading || disabled} onClick={() => void retry()}>Retry last prompt</button></div>}
      {!ready && <div className="setup-hint"><Cpu size={15} /><span>Load a local model to start a conversation.</span><button onClick={onConfigure}>Open Models <span>↗</span></button></div>}
      <form className="composer" onSubmit={event => { event.preventDefault(); void submit(); }}>
        <textarea ref={input} aria-label="Message" placeholder="Ask anything, or work on an idea…" value={draft} rows={2} maxLength={100000} onChange={e => setDraft(e.target.value)} onKeyDown={e => {
          if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); void submit(); }
        }} />
        <div className="composer-bottom"><span><span className={`status-dot ${ready ? 'ready' : ''}`} />{ready ? 'Local model' : 'No model loaded'}</span><div><small>Shift + Enter for a new line</small>{generating ? <button type="button" className="send-button" aria-label="Stop response" onClick={onCancel}><Square size={15} fill="currentColor" /></button> : <button className="send-button" type="submit" aria-label="Send message" disabled={!draft.trim() || !ready || loading || disabled}><ArrowUp size={20} /></button>}</div></div>
      </form>
      <p className="composer-note">Local inference. A space to make things happen.</p>
      {contextUsage && <p className="composer-note" aria-label="Last request context">Last request: {contextUsage.inputTokens.toLocaleString()} input + {contextUsage.responseReserve.toLocaleString()} response reserve / {contextUsage.contextLength.toLocaleString()} context tokens. Draft changes are not included.</p>}
    </div>
  </div>;
}
