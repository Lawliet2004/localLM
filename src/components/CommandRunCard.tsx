import { safeSourceUrl, sourceIdentity } from '../lib/sources';
import { useState, useEffect, useMemo, useRef, type ReactNode } from 'react';
import { ArrowDown, Check, Copy, ChevronRight, FileCode, FileText, AlertCircle, XCircle, Atom, ExternalLink, LoaderCircle } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import { parseAnsi, type AnsiLine } from '../lib/ansi';
import { TRACE_ICONS, toolKindOf, type TraceKind } from './TraceIcon';
import { CodeBlock } from './CodeBlock';
import { DiffLines, diffCounts } from './DiffView';

export function FileIcon({ filename, size = 14 }: { filename?: string; size?: number }) {
  if (!filename) return <FileCode size={size} className="cmd-icon" />;
  const lower = filename.toLowerCase();
  const ext = lower.split('.').pop() || '';
  if (ext === 'tsx' || ext === 'jsx') {
    return <Atom size={size} className="cmd-icon file-icon-react" />;
  }
  if (ext === 'ts') {
    return <span className="file-icon-ts">TS</span>;
  }
  if (ext === 'js' || ext === 'mjs' || ext === 'cjs') {
    return <span className="file-icon-js">JS</span>;
  }
  if (lower.includes('log') || ext === 'log') {
    return <FileText size={size} className="cmd-icon file-icon-log" />;
  }
  return <FileCode size={size} className="cmd-icon" />;
}

export interface CommandRunCardProps {
  toolName: string;
  command?: string;
  language?: string;
  code?: string;
  cwd?: string;
  environment?: string;
  localServerId?: string;
  status: 'running' | 'completed' | 'failed' | 'interrupted' | 'awaiting_approval';
  decision?: 'allowed' | 'denied' | string;
  isError?: boolean;
  exitCode?: number | null;
  stdout?: string;
  stderr?: string;
  diff?: string | null;
  durationMs?: number;
  error?: string | null;
  resultStatus?: string;
  envelopeStatus?: string;
  structuredResult?: string | null;
  arguments?: Record<string, unknown>;
  artifactId?: string | null;
  originalBytes?: number | null;
  authorization?: string;
  onStop?: () => void;
}

interface WebSource {
  id: string;
  title?: string;
  url?: string;
}

function parseObjectJson(value?: string | null): Record<string, unknown> | null {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Record<string, unknown> : null;
  } catch {
    return null;
  }
}

function domainOf(url?: string): string | undefined {
  if (!url) return undefined;
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return undefined;
  }
}

/** One terminal channel (stdout or stderr) with its own gutter and fold. */
function OutputChannel({ name, lines }: { name?: 'stdout' | 'stderr'; lines: AnsiLine[] }) {
  const [folded, setFolded] = useState(true);
  const foldable = lines.length > 30;
  const visible = foldable && folded ? lines.slice(0, 30) : lines;
  return (
    <div className={name ? `output-channel channel-${name}` : 'output-channel'}>
      {name && <p className="channel-label">{name}</p>}
      <div className="terminal-lines">
        {visible.map((line, idx) => (
          <div key={idx} className="terminal-line">
            <span className="terminal-gutter">{idx + 1}</span>
            <span className="terminal-content">
              {line.spans.map((span, spanIdx) => (
                <span
                  key={spanIdx}
                  style={{
                    color: span.dim ? 'var(--secondary)' : span.color,
                    fontWeight: span.bold ? 600 : 'normal',
                    fontStyle: span.italic ? 'italic' : 'normal',
                    textDecoration: span.underline ? 'underline' : 'none',
                    opacity: 1,
                  }}
                >
                  {span.text}
                </span>
              ))}
            </span>
          </div>
        ))}
      </div>
      {foldable && (
        <div className="terminal-folding-bar">
          <button type="button" className="fold-toggle-btn" onClick={() => setFolded(previous => !previous)}>
            {folded ? `Show all ${lines.length} lines` : 'Show fewer lines'}
          </button>
        </div>
      )}
    </div>
  );
}

export function webSourcesFrom(result: Record<string, unknown> | null): WebSource[] {
  const raw = result?.sources;
  if (!raw || typeof raw !== 'object') return [];
  const entries = Array.isArray(raw) ? raw.map((source, index) => [String(index + 1), source] as const) : Object.entries(raw);
  const sources = new Map<string, WebSource>();
  for (const [id, value] of entries) {
    const source = typeof value === 'string' ? { url: value } : value as Record<string, unknown> | null;
    if (!source || typeof source.url !== 'string') continue;
    const url = safeSourceUrl(source.url);
    if (!url) continue;
    const key = sourceIdentity(url);
    const prior = sources.get(key);
    sources.set(key, { id: prior?.id ?? String(source.id ?? source.sid ?? id), url: source.url, title: typeof source.title === 'string' ? source.title : typeof source.name === 'string' ? source.name : prior?.title });
  }
  return [...sources.values()];
}

export function CommandRunCard({
  toolName,
  command,
  language,
  code,
  cwd,
  environment = 'Local',
  localServerId,
  status,
  decision,
  isError,
  exitCode,
  stdout = '',
  stderr = '',
  diff,
  durationMs,
  error,
  resultStatus,
  envelopeStatus,
  structuredResult,
  arguments: args,
  artifactId,
  originalBytes,
  authorization,
  onStop,
}: CommandRunCardProps) {
  const [activeTab, setActiveTab] = useState<'terminal' | 'diff' | 'code' | 'raw' | 'result'>(diff ? 'diff' : structuredResult ? 'result' : 'terminal');
  const [copied, setCopied] = useState(false);
  const [expanded, setExpanded] = useState(status === 'running');
  const [artifactContent, setArtifactContent] = useState<string | null>(null);
  const [loadingArtifact, setLoadingArtifact] = useState(false);
  const viewportRef = useRef<HTMLDivElement>(null);
  const [scrolledUp, setScrolledUp] = useState(false);

  const path = typeof args?.path === 'string'
    ? args.path
    : typeof args?.AbsolutePath === 'string'
    ? args.AbsolutePath
    : typeof args?.TargetFile === 'string'
    ? args.TargetFile
    : undefined;

  const filename = path ? path.split(/[\\/]/).pop() : undefined;

  const startLine = args?.StartLine ?? args?.startLine ?? args?.start;
  const endLine = args?.EndLine ?? args?.endLine ?? args?.end;
  const lineRange = startLine !== undefined && endLine !== undefined ? `${startLine}-${endLine}` : undefined;

  const { added: addedCount, removed: removedCount } = diff ? diffCounts(diff) : { added: 0, removed: 0 };

  const summaryTitle = typeof args?.toolSummary === 'string'
    ? args.toolSummary
    : typeof args?.taskName === 'string'
    ? args.taskName
    : typeof args?.prompt === 'string'
    ? args.prompt
    : typeof args?.Prompt === 'string'
    ? args.Prompt
    : typeof args?.Description === 'string'
    ? args.Description
    : typeof args?.description === 'string'
    ? args.description
    : typeof args?.toolAction === 'string'
    ? args.toolAction
    : undefined;

  const isWeb = /web_search|web_open|web_find|web_fetch/.test(toolName);
  const isEdit = /edit|create_file|write_to_file|replace/.test(toolName) || Boolean(diff);
  const isSearch = isWeb || /search|grep|find|list_files|locate/.test(toolName);
  const isRead = /read_file|view_file|open_file|cat/.test(toolName) || (Boolean(path) && !isEdit && !isSearch);
  const isTask = toolName === 'manage_task';
  const isCommand = Boolean(command || code || /exec|run_code|terminal|run_command/.test(toolName));

  const kind = isWeb ? 'Web' : isEdit ? 'Edit' : isSearch ? 'Explore' : isRead ? 'Read' : isCommand ? 'Terminal' : 'Tool';

  let displayVerb = kind;
  if (isWeb) {
    displayVerb = toolName === 'web_open' ? 'Opened' : toolName === 'web_fetch_url' || toolName === 'web_fetch' ? 'Fetched' : toolName === 'web_find' ? 'Found' : 'Searched the web';
  } else if (isEdit) displayVerb = 'Edited';
  else if (isRead) displayVerb = 'Read';
  else if (isSearch) displayVerb = 'Searched';
  else if (isTask) {
    const action = typeof args?.Action === 'string' ? args.Action : '';
    displayVerb = action === 'status' || action === 'check' || !action ? 'Checked task' : 'Task';
  } else if (isCommand) {
    displayVerb = 'Ran';
  }

  const firstLine = command ? command.split(/\r?\n/)[0].trim() : '';
  const displayFilename = summaryTitle === 'Task Log'
    ? 'Task Log'
    : (filename ?? (path ? path.split(/[\\/]/).pop() : undefined));

  const searchQuery = typeof args?.Query === 'string'
    ? args.Query
    : typeof args?.query === 'string'
    ? args.query
    : typeof args?.question === 'string'
    ? args.question
    : typeof args?.Pattern === 'string'
    ? args.Pattern
    : typeof args?.url === 'string'
    ? args.url
    : typeof args?.term === 'string'
    ? args.term
    : undefined;

  const taskTitle = summaryTitle
    ?? (typeof args?.command === 'string' ? args.command : typeof args?.CommandLine === 'string' ? args.CommandLine : typeof args?.TaskId === 'string' ? args.TaskId : undefined);

  const commandTitle = summaryTitle || firstLine || `${environment} · ${toolName}`;

  const displayTitle = isTask
    ? (taskTitle || 'Task')
    : isSearch
    ? (searchQuery || 'Search')
    : isCommand
    ? (firstLine || commandTitle)
    : (displayFilename ?? (path || `${environment} · ${toolName}`));

  let searchResultCount: number | undefined = undefined;
  if (isSearch) {
    if (typeof args?.resultCount === 'number') searchResultCount = args.resultCount;
    else if (typeof (args as any)?.totalResults === 'number') searchResultCount = (args as any).totalResults;
    else {
      try {
        const parsed = typeof structuredResult === 'string' ? JSON.parse(structuredResult) : structuredResult;
        if (typeof parsed?.totalResults === 'number') searchResultCount = parsed.totalResults;
        else if (Array.isArray(parsed?.results)) searchResultCount = parsed.results.length;
        else if (Array.isArray(parsed?.matches)) searchResultCount = parsed.matches.length;
        else if (Array.isArray(parsed)) searchResultCount = parsed.length;
      } catch { /* ignore */ }
    }
    if (searchResultCount === undefined && stdout && !isWeb) {
      const trimmedLines = stdout.trim().split(/\r?\n/).filter(Boolean);
      if (trimmedLines.length > 0 && !stdout.includes('"totalResults": 0')) {
        searchResultCount = trimmedLines.length;
      }
    }
  }

  const resultObject = parseObjectJson(structuredResult) ?? parseObjectJson(stdout);
  const webSources = isWeb ? webSourcesFrom(resultObject) : [];
  const webAnswer = isWeb && resultObject && typeof resultObject.answer === 'string' ? resultObject.answer.trim() : '';
  if (isWeb) searchResultCount = webSources.length;

  const traceKind: TraceKind = toolKindOf(toolName, args ?? {}, { diff });
  const HeaderGlyph = TRACE_ICONS[traceKind];
  const headerIcon = (
    <i className={`trace-icon trace-icon-${traceKind}`} aria-hidden="true">
      {(traceKind === 'read' || traceKind === 'edit') && (path || displayFilename)
        ? <FileIcon filename={displayFilename ?? path} size={13} />
        : <HeaderGlyph size={13} />}
    </i>
  );

  const combinedOutput = stdout + (stderr ? (stdout ? '\n' : '') + stderr : '');
  const hasOutput = stdout.length > 0 || stderr.length > 0;
  // Channels parse independently so ANSI state never bleeds across streams.
  const stdoutLines = useMemo(() => (stdout ? parseAnsi(stdout) : []), [stdout]);
  const stderrLines = useMemo(() => (stderr ? parseAnsi(stderr) : []), [stderr]);

  useEffect(() => {
    if (diff && !combinedOutput && !structuredResult) {
      setActiveTab('diff');
    } else if (structuredResult) {
      setActiveTab('result');
    }
  }, [diff, combinedOutput, structuredResult]);

  useEffect(() => {
    if (status === 'running' && viewportRef.current && !scrolledUp) {
      viewportRef.current.scrollTop = viewportRef.current.scrollHeight;
    }
  }, [combinedOutput, status, scrolledUp]);

  const handleScroll = () => {
    if (!viewportRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = viewportRef.current;
    setScrolledUp(scrollHeight - scrollTop - clientHeight > 40);
  };

  const jumpToLatest = () => {
    if (!viewportRef.current) return;
    viewportRef.current.scrollTop = viewportRef.current.scrollHeight;
    setScrolledUp(false);
  };

  const copyOutput = async () => {
    try {
      await navigator.clipboard.writeText(combinedOutput || code || '');
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      /* ignore */
    }
  };

  const inspectArtifact = async () => {
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
  };

  const formatDuration = (ms?: number) => {
    if (ms === undefined || ms === null) return null;
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(1)}s`;
  };

  const renderStatusBadge = () => {
    if (status === 'running') {
      return (
        <span className="status-badge running" data-testid="status-running">
          <LoaderCircle size={13} className="spinning" />
          Running…
        </span>
      );
    }
    if (status === 'awaiting_approval') {
      return (
        <span className="status-badge awaiting" data-testid="status-awaiting">
          Awaiting Approval
        </span>
      );
    }
    if (decision === 'denied') {
      return (
        <span className="status-badge failure" data-testid="status-denied">
          <XCircle size={12} />
          Denied
        </span>
      );
    }
    if (status === 'interrupted') {
      return (
        <span className="status-badge interrupted" data-testid="status-interrupted">
          <AlertCircle size={12} />
          Stopped · outcome unknown
        </span>
      );
    }
    if (status === 'failed' || isError) {
      return (
        <span className="status-badge failure" data-testid="status-failed">
          <AlertCircle size={12} />
          Failed{exitCode != null && <span data-testid={`status-exit-${exitCode}`}>Exit {exitCode}</span>}
        </span>
      );
    }
    if (exitCode !== null && exitCode !== undefined) {
      if (exitCode === 0) {
        return (
          <span className="status-badge success" data-testid="status-exit-0">
            <Check size={14} />
            <span className="exit-text">Exit 0</span>
            <span className="finished-tag">finished</span>
          </span>
        );
      }
      return (
        <span className="status-badge failure" data-testid={`status-exit-${exitCode}`}>
          <XCircle size={12} />
          Exit {exitCode}
        </span>
      );
    }
    return (
      <span className="status-badge success" data-testid="status-finished">
        <Check size={14} />
        {isCommand ? (
          <>
            <span className="finished-text sr-only">Finished</span>
            <span className="finished-tag">finished</span>
          </>
        ) : (
          <span className="finished-text">Finished</span>
        )}
      </span>
    );
  };

  const ResultDetails = isWeb ? 'details' : 'div';
  return (
    <details
      className={`command-run-card ${status} tool-record`}
      data-testid="command-run-card"
      data-kind={traceKind}
      open={expanded}
      onToggle={e => setExpanded((e.currentTarget as HTMLDetailsElement).open)}
    >
      {/* Title Bar */}
      <summary className="card-header">
        <div className="header-left">
          {headerIcon}
          <strong className="activity-kind">{displayVerb}</strong>
          <span className={`cmd-title ${isCommand || path ? 'cmd-title-mono' : ''}`} title={path ?? displayTitle}>
            <span>{displayTitle}</span>
          </span>
          {lineRange && <span className="trace-line-range">#L{lineRange}</span>}
          {(diff || addedCount > 0 || removedCount > 0) && (
            <span className="trace-diff-counts">
              <span className="diff-added">+{addedCount}</span>{' '}
              <span className="diff-removed">−{removedCount}</span>
            </span>
          )}
          {isSearch && searchResultCount !== undefined && searchResultCount > 0 && (
            <span className="trace-badge">
              {searchResultCount} {searchResultCount === 1 ? 'result' : 'results'}
            </span>
          )}
          {cwd && <span className="cmd-cwd" title={`Working directory: ${cwd}`}>{cwd}</span>}
          {command && <span className="env-pill">{environment}</span>}
        </div>

        <div className="header-right" onClick={e => e.stopPropagation()}>
          {renderStatusBadge()}
          {status === 'running' && onStop && <button type="button" aria-label="Stop command" onClick={e => { e.preventDefault(); e.stopPropagation(); onStop(); }}>Stop</button>}

          {durationMs !== undefined && (
            <span className="duration-pill" title="Execution duration">
              {formatDuration(durationMs)}
            </span>
          )}
        </div>

        <ChevronRight size={14} className="trace-chevron" aria-hidden="true" />
      </summary>

      <div className="card-body">
        {path && path !== displayTitle && <p className="muted">{path}</p>}
        {(cwd || (command && environment) || localServerId || authorization) && (
          <p className="tool-meta">
            {[
              cwd ? <span key="cwd">Working directory <code className="tool-meta-value">{cwd}</code></span> : null,
              command && environment ? <span key="env">Environment <code className="tool-meta-value">{environment}</code></span> : null,
              localServerId ? <span key="lsid">Local server <code className="tool-meta-value">{localServerId}</code></span> : null,
              authorization ? <span key="auth">Authorization: {authorization}</span> : null,
            ].filter(Boolean).reduce<ReactNode[]>((items, item) => items.length ? [...items, <span key={`sep-${items.length}`} aria-hidden="true">·</span>, item] : [item], [])}
          </p>
        )}
        {error && <p className="error-banner" role="alert">{error}</p>}
        {(resultStatus || envelopeStatus) && (
          <p className="muted" data-testid="result-status">
            Structured result: {resultStatus ?? 'n/a'}{envelopeStatus ? ` · outcome ${envelopeStatus}` : ''}
          </p>
        )}
        {environment === 'Workspace' && toolName === 'edit_file' && (
          <p>Review the unified diff below before continuing. Re-read the file when the hash no longer matches.</p>
        )}
        {isWeb && (webAnswer || webSources.length > 0) && (
          <div className="web-result-summary">
            {webAnswer && <p className="web-answer-excerpt">{webAnswer.length > 280 ? `${webAnswer.slice(0, 280).trim()}…` : webAnswer}</p>}
            {webSources.length > 0 && (
              <ul className="web-source-list">
                {webSources.map((source, index) => {
                  const domain = domainOf(source.url);
                  const label = source.title || domain || (source.url ? source.url.replace(/^https?:\/\//, '') : source.id);
                  return (
                    <li key={source.id} className="web-source">
                      <span className="web-source-index" aria-hidden="true">{index + 1}</span>
                      <span className="web-source-main">
                        {source.url ? (
                          <a className="web-source-link" href={source.url} target="_blank" rel="noreferrer">{label}</a>
                        ) : (
                          <span className="web-source-link web-source-static">{label}</span>
                        )}
                        {source.url && domain && <span className="web-source-domain">{domain}</span>}
                      </span>
                      {source.url && <ExternalLink size={12} className="web-source-open" aria-hidden="true" />}
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        )}

        {command && (
          <div className="tool-input" data-testid="command-input">
            <CodeBlock code={command} language="bash" streaming={status === 'running'} ariaLabel={`Command: ${command}`} />
          </div>
        )}

        <ResultDetails className={isWeb ? 'web-technical-details' : 'tool-output-details'}>
          {isWeb && <summary>Raw result, logs and parameters</summary>}
        {/* Subheader tabs & actions */}
        <div className="terminal-actions-bar">
          <div className="tab-group" role="group" aria-label="Result view">
            <button
              type="button"
              aria-pressed={activeTab === 'terminal'}
                className={`tab-btn ${activeTab === 'terminal' ? 'active' : ''}`}
              onClick={() => setActiveTab('terminal')}
            >
              Output
            </button>
            {structuredResult && (
              <button
                type="button"
                aria-pressed={activeTab === 'result'}
                className={`tab-btn ${activeTab === 'result' ? 'active' : ''}`}
                onClick={() => setActiveTab('result')}
              >
                Result
              </button>
            )}
            {diff && (
              <button
                type="button"
                aria-pressed={activeTab === 'diff'}
                className={`tab-btn ${activeTab === 'diff' ? 'active' : ''}`}
                onClick={() => setActiveTab('diff')}
              >
                Diff
              </button>
            )}
            {code && (
              <button
                type="button"
                aria-pressed={activeTab === 'code'}
                className={`tab-btn ${activeTab === 'code' ? 'active' : ''}`}
                aria-label="Code" onClick={() => setActiveTab('code')}
              >
                Code{language ? ` · ${language}` : ''}
              </button>
            )}
            {args && (
              <button
                type="button"
                aria-pressed={activeTab === 'raw'}
                className={`tab-btn ${activeTab === 'raw' ? 'active' : ''}`}
                onClick={() => setActiveTab('raw')}
              >
                Parameters
              </button>
            )}
          </div>

          <div className="action-buttons">
            {combinedOutput && (
              <button
                type="button"
                className="icon-action-btn"
                onClick={() => void copyOutput()}
                title="Copy terminal output"
              >
                {copied ? <Check size={13} className="text-success" /> : <Copy size={13} />}
                <span>{copied ? 'Copied' : 'Copy'}</span>
              </button>
            )}
          </div>
        </div>

        {/* Tab 1: Monospace Terminal Viewport */}
        {activeTab === 'terminal' && (
          <div
            className="terminal-viewport"
            ref={viewportRef}
            onScroll={handleScroll}
            data-testid="terminal-viewport"
          >
            {!hasOutput ? (
              <div className="terminal-empty">
                {status === 'running' ? (
                  <span className="terminal-waiting">Waiting for command output…</span>
                ) : (
                  <span className="terminal-no-output">No output was produced.</span>
                )}
              </div>
            ) : (
              <>
                {stdoutLines.length > 0 && <OutputChannel name={stderrLines.length ? 'stdout' : undefined} lines={stdoutLines} />}
                {stderrLines.length > 0 && <OutputChannel name="stderr" lines={stderrLines} />}
              </>
            )}
            {scrolledUp && hasOutput && (
              <button type="button" className="terminal-latest" onClick={jumpToLatest} aria-label="Jump to latest output">
                <ArrowDown size={12} /> Latest
              </button>
            )}
          </div>
        )}

        {/* Tab 1b: Structured result (separate from logs) */}
        {activeTab === 'result' && structuredResult && (
          <div className="code-view" data-testid="result-viewport">
            <CodeBlock code={structuredResult} language="json" ariaLabel="Structured result" />
          </div>
        )}

        {/* Tab 2: Diff View */}
        {activeTab === 'diff' && diff && (
          <div className="diff-viewport" data-testid="diff-viewport">
            <DiffLines diff={diff} />
          </div>
        )}

        {/* Tab 3: Code View */}
        {activeTab === 'code' && code && (
          <div className="code-view" data-testid="code-viewport">
            <CodeBlock code={code} language={language} ariaLabel={`${language ?? 'code'} input`} />
          </div>
        )}

        {/* Tab 4: Raw Arguments */}
        {activeTab === 'raw' && args && (
          <div className="raw-viewport">
            <dl className="param-list">
              {Object.entries(args).map(([key, value]) => (
                <div key={key} className="param-row">
                  <dt className="param-key">{key}</dt>
                  <dd className="param-value">
                    {value !== null && typeof value === 'object' ? JSON.stringify(value, null, 2) : String(value)}
                  </dd>
                </div>
              ))}
            </dl>
          </div>
        )}

        {/* Artifact Drawer if captured */}
        {artifactId && (
          <div className="tool-artifact-box artifact-bar">
            <div className="tool-artifact-header artifact-info">
              <span>Full result captured in artifact: <code>{artifactId}</code></span>
              {typeof originalBytes === 'number' && (
                <small>({(originalBytes / 1024).toFixed(1)} KB)</small>
              )}
              <button
                type="button"
                className="secondary text-btn artifact-inspect-btn"
                onClick={() => void inspectArtifact()}
                disabled={loadingArtifact}
              >
                {loadingArtifact ? 'Loading…' : artifactContent !== null ? 'Loaded' : 'Inspect artifact'}
              </button>
            </div>
            {artifactContent !== null && (
              <pre className="tool-artifact-preview artifact-preview">{artifactContent}</pre>
            )}
          </div>
        )}
        </ResultDetails>
      </div>
    </details>
  );
}
