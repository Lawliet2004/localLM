import { useState, useEffect, useRef } from 'react';
import { Check, Copy, ChevronDown, ChevronRight, FileCode, FileText, AlertCircle, Clock, XCircle, Atom } from 'lucide-react';
import { api, errorMessage } from '../lib/api';

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

const ANSI_COLORS: Record<number, string> = {
  30: 'var(--terminal-black, #4b5563)',
  31: 'var(--terminal-red, #ef4444)',
  32: 'var(--terminal-green, #10b981)',
  33: 'var(--terminal-yellow, #f59e0b)',
  34: 'var(--terminal-blue, #3b82f6)',
  35: 'var(--terminal-magenta, #ec4899)',
  36: 'var(--terminal-cyan, #06b6d4)',
  37: 'var(--terminal-white, #f3f4f6)',
  90: 'var(--terminal-bright-black, #6b7280)',
  91: 'var(--terminal-bright-red, #f87171)',
  92: 'var(--terminal-bright-green, #34d399)',
  93: 'var(--terminal-bright-yellow, #fbbf24)',
  94: 'var(--terminal-bright-blue, #60a5fa)',
  95: 'var(--terminal-bright-magenta, #f472b6)',
  96: 'var(--terminal-bright-cyan, #22d3ee)',
  97: 'var(--terminal-bright-white, #ffffff)',
};

interface AnsiSpan {
  text: string;
  color?: string;
  bold?: boolean;
  dim?: boolean;
}

function parseAnsiLine(line: string): AnsiSpan[] {
  const regex = /\x1b\[([0-9;]*)m/g;
  const spans: AnsiSpan[] = [];
  let lastIndex = 0;
  let currentColor: string | undefined = undefined;
  let currentBold = false;
  let currentDim = false;

  let match: RegExpExecArray | null;
  while ((match = regex.exec(line)) !== null) {
    if (match.index > lastIndex) {
      spans.push({
        text: line.slice(lastIndex, match.index),
        color: currentColor,
        bold: currentBold,
        dim: currentDim,
      });
    }

    const codes = match[1] ? match[1].split(';').map(Number) : [0];
    for (const code of codes) {
      if (code === 0) {
        currentColor = undefined;
        currentBold = false;
        currentDim = false;
      } else if (code === 1) {
        currentBold = true;
      } else if (code === 2) {
        currentDim = true;
      } else if (code === 22) {
        currentBold = false;
        currentDim = false;
      } else if (code === 39) {
        currentColor = undefined;
      } else if (ANSI_COLORS[code]) {
        currentColor = ANSI_COLORS[code];
      }
    }
    lastIndex = regex.lastIndex;
  }

  if (lastIndex < line.length) {
    spans.push({
      text: line.slice(lastIndex),
      color: currentColor,
      bold: currentBold,
      dim: currentDim,
    });
  }

  return spans.length > 0 ? spans : [{ text: line }];
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
  const [isFolded, setIsFolded] = useState(true);
  const [artifactContent, setArtifactContent] = useState<string | null>(null);
  const [loadingArtifact, setLoadingArtifact] = useState(false);
  const viewportRef = useRef<HTMLDivElement>(null);
  const userScrolledUp = useRef(false);

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

  let addedCount = 0;
  let removedCount = 0;
  if (diff) {
    let inHunk = false;
    for (const line of diff.split('\n')) {
      if (line.startsWith('@@')) inHunk = true;
      else if (inHunk && line.startsWith('+')) addedCount++;
      else if (inHunk && line.startsWith('-')) removedCount++;
    }
  }

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

  const isEdit = /edit|create_file|write_to_file|replace/.test(toolName) || Boolean(diff);
  const isSearch = /search|grep|find|list_files|locate/.test(toolName);
  const isRead = /read_file|view_file|open_file|cat/.test(toolName) || (Boolean(path) && !isEdit && !isSearch);
  const isTask = toolName === 'manage_task';
  const isCommand = Boolean(command || code || /exec|run_code|terminal|run_command/.test(toolName));

  const kind = isEdit ? 'Edit' : isSearch ? 'Explore' : isRead ? 'Read' : isCommand ? 'Terminal' : 'Tool';

  let displayVerb = kind;
  if (isEdit) displayVerb = 'Edited';
  else if (isRead) displayVerb = 'Analyzed';
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
    : typeof args?.Pattern === 'string'
    ? args.Pattern
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
    if (searchResultCount === undefined && stdout) {
      const trimmedLines = stdout.trim().split(/\r?\n/).filter(Boolean);
      if (trimmedLines.length > 0 && !stdout.includes('"totalResults": 0')) {
        searchResultCount = trimmedLines.length;
      }
    }
  }

  const renderHeaderIcon = () => {
    if (path || isRead || isEdit || displayFilename) return <FileIcon filename={displayFilename ?? path} size={14} />;
    return null;
  };

  const combinedOutput = stdout + (stderr ? (stdout ? '\n' : '') + stderr : '');
  const lines = combinedOutput ? combinedOutput.split(/\r?\n/) : [];
  const shouldAllowFolding = lines.length > 30;
  const visibleLines = shouldAllowFolding && isFolded ? lines.slice(0, 30) : lines;

  useEffect(() => {
    if (diff && !combinedOutput && !structuredResult) {
      setActiveTab('diff');
    } else if (structuredResult) {
      setActiveTab('result');
    }
  }, [diff, combinedOutput, structuredResult]);

  useEffect(() => {
    if (status === 'running' && viewportRef.current && !userScrolledUp.current) {
      viewportRef.current.scrollTop = viewportRef.current.scrollHeight;
    }
  }, [combinedOutput, status]);

  const handleScroll = () => {
    if (!viewportRef.current) return;
    const { scrollTop, scrollHeight, clientHeight } = viewportRef.current;
    userScrolledUp.current = scrollHeight - scrollTop - clientHeight > 40;
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
          <span className="pulse-dot" />
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
    if (exitCode !== null && exitCode !== undefined) {
      if (exitCode === 0) {
        return (
          <span className="status-badge success" data-testid="status-exit-0">
            <span className="exit-text">Exit 0</span>
            <span className="finished-tag">finished</span>
            <ChevronRight size={13} className="finished-chevron" />
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
          Failed
        </span>
      );
    }
    return (
      <span className="status-badge success" data-testid="status-finished">
        {isCommand ? (
          <>
            <span className="finished-text sr-only">Finished</span>
            <span className="finished-tag">finished</span>
            <ChevronRight size={13} className="finished-chevron" />
          </>
        ) : (
          <span className="finished-text">Finished</span>
        )}
      </span>
    );
  };

  return (
    <details
      className={`command-run-card ${status} tool-record`}
      data-testid="command-run-card"
      open={expanded}
      onToggle={e => setExpanded((e.currentTarget as HTMLDetailsElement).open)}
    >
      {/* Title Bar */}
      <summary className="card-header">
        <div className="header-left">
          <span className="collapse-btn sr-only" aria-label={expanded ? 'Collapse output' : 'Expand output'}>
            {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </span>
          {!(isCommand && status === 'completed') && <strong className="activity-kind">{displayVerb}</strong>}
          {renderHeaderIcon()}
          <span className="cmd-title" title={path ?? displayTitle}>
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
              <Clock size={11} />
              {formatDuration(durationMs)}
            </span>
          )}
        </div>
      </summary>

      <div className="card-body">
        {path && path !== displayTitle && <p className="muted">{path}</p>}
        {error && <p className="error-banner" role="alert">{error}</p>}
        {(resultStatus || envelopeStatus) && (
          <p className="muted" data-testid="result-status">
            Structured result: {resultStatus ?? 'n/a'}{envelopeStatus ? ` · outcome ${envelopeStatus}` : ''}
          </p>
        )}
        {localServerId && <p>Local server ID: <code>{localServerId}</code></p>}
        {authorization && <p>Authorization: {authorization}</p>}
        {environment === 'Workspace' && toolName === 'edit_file' && (
          <p>Review the unified diff below before continuing. Re-read the file when the hash no longer matches.</p>
        )}

        {/* Subheader tabs & actions */}
        <div className="terminal-actions-bar">
          <div className="tab-group">
            <button
              type="button"
              className={`tab-btn ${activeTab === 'terminal' ? 'active' : ''}`}
              onClick={() => setActiveTab('terminal')}
            >
              Output
            </button>
            {structuredResult && (
              <button
                type="button"
                className={`tab-btn ${activeTab === 'result' ? 'active' : ''}`}
                onClick={() => setActiveTab('result')}
              >
                Result
              </button>
            )}
            {diff && (
              <button
                type="button"
                className={`tab-btn ${activeTab === 'diff' ? 'active' : ''}`}
                onClick={() => setActiveTab('diff')}
              >
                Diff
              </button>
            )}
            {code && (
              <button
                type="button"
                className={`tab-btn ${activeTab === 'code' ? 'active' : ''}`}
                aria-label="Code" onClick={() => setActiveTab('code')}
              >
                Code{language ? ` · ${language}` : ''}
              </button>
            )}
            {args && (
              <button
                type="button"
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
            {visibleLines.length === 0 ? (
              <div className="terminal-empty">
                {status === 'running' ? (
                  <span className="terminal-waiting">Waiting for command output…</span>
                ) : (
                  <span className="terminal-no-output">No output was produced.</span>
                )}
              </div>
            ) : (
              <div className="terminal-lines">
                {visibleLines.map((line, idx) => (
                  <div key={idx} className="terminal-line">
                    <span className="terminal-gutter">{idx + 1}</span>
                    <span className="terminal-content">
                      {parseAnsiLine(line).map((span, spanIdx) => (
                        <span
                          key={spanIdx}
                          style={{
                            color: span.color,
                            fontWeight: span.bold ? 600 : 'normal',
                            opacity: span.dim ? 0.7 : 1,
                          }}
                        >
                          {span.text}
                        </span>
                      ))}
                    </span>
                  </div>
                ))}
              </div>
            )}

            {shouldAllowFolding && (
              <div className="terminal-folding-bar">
                <button
                  type="button"
                  className="fold-toggle-btn"
                  onClick={() => setIsFolded(prev => !prev)}
                >
                  {isFolded ? `Show all ${lines.length} lines` : 'Show fewer lines'}
                </button>
              </div>
            )}
          </div>
        )}

        {/* Tab 1b: Structured result (separate from logs) */}
        {activeTab === 'result' && structuredResult && (
          <div className="terminal-viewport" data-testid="result-viewport">
            <pre className="tool-result-json">{structuredResult}</pre>
          </div>
        )}

        {/* Tab 2: Diff View */}
        {activeTab === 'diff' && diff && (
          <div className="diff-viewport" data-testid="diff-viewport">
            <pre className="tool-diff diff-lines">
              {diff.split('\n').map((line, idx) => {
                const isAdd = line.startsWith('+') && !line.startsWith('+++');
                const isDel = line.startsWith('-') && !line.startsWith('---');
                const isHdr = line.startsWith('@@');
                const lineClass = isAdd ? 'diff-line add' : isDel ? 'diff-line del' : isHdr ? 'diff-line hdr' : 'diff-line';
                return (
                  <div key={idx} className={lineClass}>
                    <span className="diff-gutter">{idx + 1}</span>
                    <span className="diff-content">{line}</span>
                  </div>
                );
              })}
            </pre>
          </div>
        )}

        {/* Tab 3: Code View */}
        {activeTab === 'code' && code && (
          <div className="terminal-viewport" data-testid="code-viewport">
            <div className="terminal-lines">
              {code.split(/\r?\n/).map((line, idx) => (
                <div key={idx} className="terminal-line">
                  <span className="terminal-gutter">{idx + 1}</span>
                  <span className="terminal-content">
                    <code>{line}</code>
                  </span>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Tab 4: Raw Arguments */}
        {activeTab === 'raw' && args && (
          <div className="raw-viewport">
            <pre>{JSON.stringify(args, null, 2)}</pre>
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
                className="secondary text-btn"
                style={{ fontSize: '11px', padding: '2px 8px', marginLeft: 'auto' }}
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
      </div>
    </details>
  );
}
