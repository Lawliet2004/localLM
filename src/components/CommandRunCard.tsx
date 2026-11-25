import { safeSourceUrl, sourceIdentity } from '../lib/sources';
import { memo, useState, useEffect, useMemo, useRef } from 'react';
import { ArrowDown, Check, Copy, ChevronRight, AlertCircle, XCircle, ExternalLink } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import { AccumulatedAnsiParser, parseAnsi, type AnsiLine } from '../lib/ansi';
import { toolKindOf } from './TraceIcon';
import { CodeBlock } from './CodeBlock';
import { DiffLines, diffCounts } from './DiffView';

export interface CommandRunCardProps {
  toolName: string;
  command?: string;
  language?: string;
  code?: string;
  cwd?: string;
  environment?: string;
  localServerId?: string;
  status: 'running' | 'completed' | 'failed' | 'interrupted' | 'awaiting_approval';
  lifecycleState?: string;
  decision?: 'allowed' | 'denied' | string;
  isError?: boolean;
  exitCode?: number | null;
  stdout?: string;
  stderr?: string;
  diff?: string | null;
  durationMs?: number;
  error?: string | null;
  errorCode?: string;
  resultStatus?: string;
  envelopeStatus?: string;
  resultPath?: string;
  structuredResult?: string | null;
  diagnosticResult?: Record<string, unknown> | null;
  arguments?: Record<string, unknown>;
  artifactId?: string | null;
  originalBytes?: number | null;
  authorization?: string;
  onStop?: () => void;
  stopLabel?: string;
}

interface WebSource {
  id: string;
  title?: string;
  url?: string;
}

type ResultView = 'terminal' | 'diff' | 'code' | 'raw' | 'result';

const localToolLabels = new Map([
  ['artifact_read', 'Artifact excerpt'],
  ['development_guide', 'Development guide'],
  ['check_run', 'Run checks'],
  ['check_report', 'Check report'],
  ['verification_run', 'Run verification'],
  ['verification_report', 'Verification report'],
]);

function parseJson(value?: string | null): unknown {
  if (!value) return null;
  try { return JSON.parse(value); } catch { return null; }
}

function objectRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

function CommandOutputExcerpt({ output, stderr = false }: { output: string; stderr?: boolean }) {
  // The full inspector uses the incremental parser. A closed row only needs a
  // few lines and should never parse all of a large saved log.
  const limit = 8192;
  let start = Math.max(0, output.length - limit);
  let end = output.length;
  if (start > 0) {
    const boundary = output.indexOf('\n', start);
    if (boundary >= 0 && boundary < end - 1) start = boundary + 1;
    else {
      start = output.lastIndexOf('\n', start - 1) + 1;
      end = start + limit;
    }
  }
  const lines = parseAnsi(output.slice(start, end)).flatMap(line => {
    const text = line.spans.map(span => span.text).join('').trimEnd();
    return text ? [text] : [];
  }).slice(stderr ? -2 : -6);
  if (!lines.length) return null;
  return <div className="command-output-excerpt">
    {stderr && <span className="excerpt-channel">stderr</span>}
    <pre aria-label={stderr ? 'Recent stderr' : 'Recent command output'}>
      {lines.map((line, index) => <code className={/^\s*[✓✔]/.test(line) ? 'output-check' : undefined} key={index}>{line.length > 200 ? `${line.slice(0, 200)}…` : line}</code>)}
    </pre>
  </div>;
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
const OutputChannel = memo(function OutputChannel({ name, lines, live, following }: { name?: 'stdout' | 'stderr'; lines: AnsiLine[]; live: boolean; following: boolean }) {
  // Folded: the last 30 lines, following the stream. Expanded: one scrollable
  // view of the most recent lines, capped so very long output stays responsive.
  const [folded, setFolded] = useState(true);
  const foldable = lines.length > 30;
  const expandedCap = 2000;
  const tail = useRef(0);
  const hasStreamed = useRef(live);
  if (live) hasStreamed.current = true;
  if (hasStreamed.current && following) tail.current = Math.max(0, lines.length - 30);
  const foldedOffset = Math.min(tail.current, Math.max(0, lines.length - 30));
  const expandedOffset = Math.max(0, lines.length - expandedCap);
  const offset = foldable && folded ? foldedOffset : expandedOffset;
  const visible = foldable && folded ? lines.slice(offset, offset + 30) : lines.slice(offset);
  return (
    <div className={name ? `output-channel channel-${name}` : 'output-channel'}>
      {name && <p className="channel-label">{name}</p>}
      <div className="terminal-lines" role="region" aria-label={name ? `${name} output` : 'Command output'} tabIndex={0}>
        {visible.map((line, idx) => (
          <div key={offset + idx} className="terminal-line">
            <span className="terminal-gutter">{offset + idx + 1}</span>
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
          {!folded && lines.length > expandedCap && <span className="muted">Showing the last {expandedCap} of {lines.length} lines</span>}
        </div>
      )}
    </div>
  );
});

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
  lifecycleState,
  isError,
  exitCode,
  stdout = '',
  stderr = '',
  diff,
  durationMs,
  error,
  errorCode,
  resultStatus,
  envelopeStatus,
  resultPath,
  structuredResult,
  diagnosticResult,
  arguments: args,
  artifactId,
  originalBytes,
  authorization,
  onStop,
  stopLabel = 'Stop response',
}: CommandRunCardProps) {
  const [selectedView, setSelectedView] = useState<ResultView>();
  const [copied, setCopied] = useState<'command' | 'output'>();
  const [copyError, setCopyError] = useState(false);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(copyTimer.current), []);
  const [expanded, setExpanded] = useState(status === 'failed' || status === 'awaiting_approval');
  const [inspected, setInspected] = useState(status === 'failed' || status === 'awaiting_approval');
  const [inspectedMetadata, setInspectedMetadata] = useState(false);
  const [artifactContent, setArtifactContent] = useState<string | null>(null);
  const [loadingArtifact, setLoadingArtifact] = useState(false);
  const viewportRef = useRef<HTMLDivElement>(null);
  const [scrolledUp, setScrolledUp] = useState(false);
  // A card opened while running stays open after it completes, so the output
  // the user watched is still there. Failed and waiting cards open on their own.
  useEffect(() => {
    if (status === 'failed' || status === 'awaiting_approval') { setExpanded(true); setInspected(true); }
  }, [status]);

  const path = typeof args?.path === 'string'
    ? args.path
    : typeof args?.AbsolutePath === 'string'
    ? args.AbsolutePath
    : typeof args?.TargetFile === 'string'
    ? args.TargetFile
    : resultPath;

  const filename = path ? path.split(/[\\/]/).pop() : undefined;

  const startLine = args?.StartLine ?? args?.startLine ?? args?.start;
  const endLine = args?.EndLine ?? args?.endLine ?? args?.end;
  const lineRange = startLine !== undefined && endLine !== undefined ? `${startLine}-${endLine}` : undefined;

  const { added: addedCount, removed: removedCount } = useMemo(() => diff ? diffCounts(diff) : { added: 0, removed: 0 }, [diff]);

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

  const category = toolKindOf(toolName, { ...args, ...(command ? { command } : {}), ...(code ? { code } : {}) }, diff ? { diff } : {});
  const isWeb = category === 'web';
  const isEdit = category === 'edit';
  const isSearch = isWeb || category === 'search' || category === 'folder';
  const isRead = category === 'read';
  const isTask = category === 'task';
  const isCommand = category === 'command';
  const isDirectory = category === 'folder' && toolName !== 'project_inspect';

  const kind = isWeb ? 'Web' : isEdit ? 'Edit' : isSearch ? 'Explore' : isRead ? 'Read' : isCommand ? 'Terminal' : 'Tool';

  const running = status === 'running';
  let displayVerb = running ? 'Running tool' : kind;
  const unsettled = status === 'awaiting_approval' || status === 'failed' || status === 'interrupted';
  if (isWeb) {
    displayVerb = unsettled ? 'Browse' : status === 'running' ? 'Browsing' : ['web_open', 'visit'].includes(toolName) ? 'Opened' : toolName === 'web_fetch_url' || toolName === 'web_fetch' ? 'Fetched' : toolName === 'web_find' ? 'Found' : 'Searched the web';
  } else if (isEdit) displayVerb = unsettled ? 'Edit' : status === 'running' ? 'Editing' : 'Edited';
  else if (isRead) displayVerb = status === 'running' ? 'Reading file' : 'Read file';
  else if (isDirectory) displayVerb = unsettled ? 'List directory' : status === 'running' ? 'Listing directory' : 'Listed directory';
  else if (isSearch) displayVerb = unsettled ? 'Search files' : status === 'running' ? 'Searching files' : 'Searched files';
  else if (isTask) {
    const action = typeof args?.Action === 'string' ? args.Action : '';
    displayVerb = running ? 'Checking task' : action === 'status' || action === 'check' || !action ? 'Checked task' : 'Task';
  } else if (isCommand) {
    displayVerb = status === 'running' ? 'Running' : unsettled ? 'Run' : 'Ran';
  }

  const displayFilename = summaryTitle === 'Task Log'
    ? 'Task Log'
    : (filename ?? (path ? path.split(/[\\/]/).pop() : undefined));

  const searchQuery = typeof args?.Query === 'string'
    ? args.Query
    : typeof args?.query === 'string'
    ? args.query
    : Array.isArray(args?.query) && args.query.every(value => typeof value === 'string')
    ? args.query.join(' · ')
    : typeof args?.question === 'string'
    ? args.question
    : typeof args?.Pattern === 'string'
    ? args.Pattern
    : typeof args?.pattern === 'string'
    ? args.pattern
    : typeof args?.url === 'string'
    ? args.url
    : Array.isArray(args?.url) && args.url.every(value => typeof value === 'string')
    ? args.url.join(' · ')
    : typeof args?.term === 'string'
    ? args.term
    : undefined;

  const taskTitle = summaryTitle
    ?? (typeof args?.command === 'string' ? args.command : typeof args?.CommandLine === 'string' ? args.CommandLine : typeof args?.TaskId === 'string' ? args.TaskId : undefined);

  const localLabel = environment === 'Local' && !localServerId ? localToolLabels.get(toolName) : undefined;
  const toolTitle = localLabel ?? `${environment} · ${toolName}`;
  const commandTitle = summaryTitle || toolTitle;

  const displayTitle = isTask
    ? (taskTitle || 'Task')
    : isSearch
    ? (isDirectory ? path || searchQuery || '.' : searchQuery || path || toolTitle)
    : isCommand
    ? commandTitle
    : (path || displayFilename || toolTitle);

  const searchResult = useMemo(() => isSearch ? parseJson(structuredResult) ?? (isWeb ? parseJson(stdout) : null) : null,
    [isSearch, isWeb, stdout, structuredResult]);
  const resultObject = objectRecord(searchResult);
  const webSources = isWeb ? webSourcesFrom(resultObject) : [];
  const webAnswer = isWeb && resultObject && typeof resultObject.answer === 'string' ? resultObject.answer.trim() : '';
  const searchEntries: unknown[] | undefined = [resultObject?.matches, resultObject?.results, resultObject?.entries, searchResult].find(Array.isArray);
  const partialSearch = Boolean(resultObject?.hasMore || resultObject?.incomplete || resultObject?.truncated);
  const searchResultCount = isWeb && resultObject?.sources ? webSources.length
    : (partialSearch ? searchEntries?.length : undefined) ??
      [args?.resultCount, args?.totalResults, resultObject?.totalResults, searchEntries?.length].find(
        (value): value is number => typeof value === 'number' && Number.isFinite(value) && value >= 0,
      );
  const searchTargets = isWeb ? webSources.slice(0, 2).map(source => source.title || domainOf(source.url) || source.id)
    : searchEntries?.slice(0, 3).flatMap(entry => {
      if (typeof entry === 'string') return [entry];
      const match = objectRecord(entry);
      if (typeof match?.path === 'string') return [typeof match.line === 'number' ? `${match.path}:${match.line}` : match.path];
      return typeof match?.name === 'string' ? [match.name] : [];
    }) ?? [];

  const operation = command ? 'Bash'
    : localLabel ?? (isWeb ? ['web_open', 'visit', 'web_fetch_url', 'web_fetch'].includes(toolName) ? 'WebOpen' : 'WebSearch'
      : isRead ? 'Read' : isEdit ? 'Update' : isDirectory ? 'List' : isSearch ? 'Search' : isCommand ? 'Terminal' : toolName);
  const target = command ?? (displayTitle === operation ? '' : displayTitle === toolTitle && operation === toolName ? environment : displayTitle);
  const accessibleTarget = operation === toolName ? `${operation}${target ? ` ${target}` : ''}` : target || operation;

  const combinedOutput = stdout + (stderr ? (stdout ? '\n' : '') + stderr : '');
  const hasOutput = combinedOutput.length > 0;
  const views: { id: ResultView; label: string }[] = [];
  if (hasOutput) views.push({ id: 'terminal', label: 'Output' });
  if (structuredResult) views.push({ id: 'result', label: 'Result' });
  if (diff) views.push({ id: 'diff', label: 'Diff' });
  if (code) views.push({ id: 'code', label: 'Code' });
  if (args && Object.keys(args).length > 0) views.push({ id: 'raw', label: 'Parameters' });
  const preferredView = structuredResult ? 'result' : diff ? 'diff' : views.find(view => view.id !== 'raw')?.id;
  const activeTab = views.some(view => view.id === selectedView) ? selectedView : preferredView;
  const duration = formatDuration(durationMs);
  const hasMetadata = Boolean(localServerId || authorization || cwd || resultStatus || envelopeStatus || errorCode || localLabel || diagnosticResult);
  const hasResultDetails = views.length > 0 || Boolean(artifactId);
  const hasDetails = hasResultDetails || hasMetadata || Boolean(error || (path && path !== displayTitle) || webAnswer || webSources.length);
  const detailsOpen = expanded && hasDetails;
  // Channels parse independently so ANSI state never bleeds across streams.
  const stdoutParser = useRef(new AccumulatedAnsiParser());
  const stderrParser = useRef(new AccumulatedAnsiParser());
  const stdoutSnapshot = useRef<AnsiLine[]>([]);
  const stderrSnapshot = useRef<AnsiLine[]>([]);
  const stdoutLines = useMemo(() => { if (detailsOpen && activeTab === 'terminal') stdoutSnapshot.current = stdoutParser.current.update(stdout); return stdoutSnapshot.current; }, [stdout, detailsOpen, activeTab]);
  const stderrLines = useMemo(() => { if (detailsOpen && activeTab === 'terminal') stderrSnapshot.current = stderrParser.current.update(stderr); return stderrSnapshot.current; }, [stderr, detailsOpen, activeTab]);

  useEffect(() => {
    if (viewportRef.current && !scrolledUp) {
      viewportRef.current.scrollTop = viewportRef.current.scrollHeight;
    }
  }, [combinedOutput, status, scrolledUp, detailsOpen, activeTab]);

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

  const copyText = async (target: 'command' | 'output', text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopyError(false); setCopied(target);
      clearTimeout(copyTimer.current);
      copyTimer.current = setTimeout(() => setCopied(undefined), 2000);
    } catch {
      setCopyError(true);
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

  function formatDuration(ms?: number) {
    if (ms === undefined || ms === null || !Number.isFinite(ms) || ms < 0) return null;
    if (ms < 1000) return `${Math.round(ms)}ms`;
    return `${(ms / 1000).toFixed(1)}s`;
  }

  const renderStatusBadge = () => {
    if (running) {
      return localLabel && !command ? <span className="status-badge running">Running…</span> : null;
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
    if (lifecycleState === 'stopped' || lifecycleState === 'unknown') {
      return <span className="status-badge interrupted" data-testid="status-interrupted"><AlertCircle size={12} />{lifecycleState === 'stopped' ? 'Stopped' : 'Process outcome unknown'}</span>;
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
          {lifecycleState === 'timed_out' || errorCode === 'timeout' ? 'Timed out' : 'Failed'}
          {exitCode != null && <span data-testid={`status-exit-${exitCode}`}><span aria-hidden="true"> · </span>Exit {exitCode}</span>}
        </span>
      );
    }
    if (exitCode !== null && exitCode !== undefined) {
      if (exitCode === 0) {
        return (
          <span className="status-badge success" data-testid="status-exit-0">
            <Check size={14} />
            <span className="exit-text">Exit 0</span>
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
        <span className="finished-text">Finished</span>
      </span>
    );
  };

  const ResultDetails = isWeb ? 'details' : 'div';
  const showDiffExcerpt = status === 'completed' && diff && (addedCount > 0 || removedCount > 0);
  const searchSummary = isSearch && status === 'completed' && searchResultCount !== undefined
    ? `${searchResultCount} ${searchResultCount === 1 ? 'result' : 'results'}${partialSearch ? ' shown' : ''}` : null;
  return <>
    <details
      className={`command-run-card ${status} tool-record`}
      data-testid="command-run-card"
      data-kind={category}
      open={detailsOpen}
      onToggle={e => { const open = e.currentTarget.open; setExpanded(open); if (open) setInspected(true); }}
    >
      {/* Title Bar */}
      <summary className="card-header" aria-label={`${displayVerb} ${accessibleTarget}`} onClick={event => {
        if ((event.target as Element).closest('button')) return;
        event.preventDefault();
        if (hasDetails) { setInspected(true); setExpanded(!detailsOpen); }
      }}>
        <div className="header-left">
          <span className="tool-dot" aria-hidden="true" />
          <span className={`activity-entry-label${running ? ' activity-sweep' : ''}`} data-testid={running ? 'status-running' : undefined}>
            <strong className="activity-kind">{operation}</strong>
            {target && <span className="tool-target">(<code className={command ? 'command-preview' : 'cmd-title'} title={target}>{target}</code>)</span>}
          </span>
        </div>

        <div className="header-right">
          {lineRange && <span className="trace-line-range">#L{lineRange}</span>}
          {(diff || addedCount > 0 || removedCount > 0) && (
            <span className="trace-diff-counts">
              <span className="diff-added">+{addedCount}</span>{' '}
              <span className="diff-removed">−{removedCount}</span>
            </span>
          )}
          {renderStatusBadge()}
          {status === 'running' && onStop && <button type="button" aria-label={stopLabel} onClick={e => { e.preventDefault(); e.stopPropagation(); onStop(); }}>Stop</button>}

          {duration && (
            <span className="duration-pill" title="Execution duration">
              {duration}
            </span>
          )}
          {command && <button type="button" className="icon-action-btn command-copy"
            aria-label={copied === 'command' ? 'Command copied' : 'Copy command'}
            onClick={event => { event.stopPropagation(); void copyText('command', command); }}>
            {copied === 'command' ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
          </button>}
        </div>

        {hasDetails && <ChevronRight size={14} className="trace-chevron" aria-hidden="true" />}
      </summary>

      {hasDetails && (detailsOpen || inspected) && <div className="card-body">
        {error && <p className="tool-error" role="alert"><AlertCircle size={15} aria-hidden="true" /><span>{error}</span></p>}
        {path && path !== displayTitle && <p className="muted">{path}</p>}
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

        {hasResultDetails && <ResultDetails className={isWeb ? 'web-technical-details' : 'tool-output-details'}>
          {isWeb && <summary>Technical result</summary>}
          {views.length > 0 && <div className="terminal-actions-bar">
            <div className="tab-group" role="group" aria-label="Result view">
              {views.map(({ id, label }) => (
                <button key={id} type="button" aria-pressed={activeTab === id}
                  className={`tab-btn ${activeTab === id ? 'active' : ''}`}
                  onClick={() => setSelectedView(current => id === 'raw' && current === id ? undefined : id)}>
                  {label}
                </button>
              ))}
            </div>

            <div className="action-buttons">
              {activeTab === 'terminal' && hasOutput && (
                <button
                  type="button"
                  className="icon-action-btn"
                  onClick={() => void copyText('output', combinedOutput)}
                  title="Copy terminal output"
                >
                  {copied === 'output' ? <Check size={13} className="text-success" /> : <Copy size={13} />}
                  <span>{copied === 'output' ? 'Copied' : 'Copy'}</span>
                </button>
              )}
            </div>
          </div>}

          {activeTab === 'terminal' && (
            <div
              className="terminal-viewport"
              ref={viewportRef}
              onScroll={handleScroll}
              data-testid="terminal-viewport"
            >
              {stdoutLines.length > 0 && <OutputChannel name={stderrLines.length ? 'stdout' : undefined} lines={stdoutLines} live={running} following={!scrolledUp} />}
              {stderrLines.length > 0 && <OutputChannel name="stderr" lines={stderrLines} live={running} following={!scrolledUp} />}
              {scrolledUp && hasOutput && (
                <button type="button" className="terminal-latest" onClick={jumpToLatest} aria-label="Jump to latest output">
                  <ArrowDown size={12} /> Latest
                </button>
              )}
            </div>
          )}

          {activeTab === 'result' && structuredResult && (
            <div className="code-view" data-testid="result-viewport">
              <CodeBlock code={structuredResult} language="json" ariaLabel="Structured result" />
            </div>
          )}

          {activeTab === 'diff' && diff && (
            <div className="diff-viewport" data-testid="diff-viewport">
              <DiffLines diff={diff} path={path} />
            </div>
          )}

          {activeTab === 'code' && code && (
            <div className="code-view" data-testid="code-viewport">
              <CodeBlock code={code} language={language} ariaLabel={`${language ?? 'code'} input`} />
            </div>
          )}

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
        </ResultDetails>}
        {hasMetadata && <details className="tool-metadata" onToggle={event => {
          if (event.currentTarget.open) setInspectedMetadata(true);
        }}>
          <summary><ChevronRight size={13} aria-hidden="true" />Execution details</summary>
          <div className="tool-meta">
            {localLabel && <span>Tool <code className="tool-meta-value">{toolName}</code></span>}
            {localServerId && <span>Local server <code className="tool-meta-value">{localServerId}</code></span>}
            {cwd && <span>Working directory <code className="tool-meta-value">{cwd}</code></span>}
            {authorization && <span>Authorization: {authorization}</span>}
            {(resultStatus || envelopeStatus) && <span data-testid="result-status">
              {resultStatus ? `Result: ${resultStatus}${envelopeStatus ? ` · ${envelopeStatus}` : ''}` : `Status: ${envelopeStatus}`}
            </span>}
            {errorCode && <span>Error code <code className="tool-meta-value">{errorCode}</code></span>}
          </div>
          {inspectedMetadata && diagnosticResult && <div className="code-view">
            <CodeBlock code={JSON.stringify(diagnosticResult, null, 2)} language="json" ariaLabel="Raw tool result" />
          </div>}
        </details>}
      </div>}
      {copyError && <p role="alert" className="code-block-copy-error">Clipboard access failed.</p>}
    </details>
    {!detailsOpen && showDiffExcerpt && <div className="activity-excerpt diff-excerpt">
      <p className="transcript-branch">Updated with {addedCount} {addedCount === 1 ? 'addition' : 'additions'}{removedCount > 0 ? ` and ${removedCount} ${removedCount === 1 ? 'removal' : 'removals'}` : ''}</p>
      <DiffLines diff={diff} path={path} excerpt />
    </div>}
    {!detailsOpen && searchSummary && <div className="activity-excerpt">
      <p className="transcript-branch">{searchSummary}</p>
      {searchTargets.length > 0 && <ul className="search-excerpt" aria-label="Result excerpt">
        {searchTargets.map((target, index) => {
          const label = target.length > 240 ? `${target.slice(0, 240)}…` : target;
          return <li key={index}>{isWeb ? label : <code>{label}</code>}</li>;
        })}
      </ul>}
    </div>}
    {!detailsOpen && isCommand && hasOutput && <div className="activity-excerpt transcript-branch">
      {stdout && <CommandOutputExcerpt output={stdout} />}
      {stderr && <CommandOutputExcerpt output={stderr} stderr />}
    </div>}
  </>;
}
