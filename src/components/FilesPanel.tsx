import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChevronRight, Copy, File, Folder, FolderOpen, FolderSearch, Search } from 'lucide-react';
import { openPath } from '@tauri-apps/plugin-opener';
import hljs from 'highlight.js/lib/common';
import { api, errorMessage } from '../lib/api';

export interface FsNode {
  name: string;
  kind: string;
  path: string;
  expanded: boolean;
  loaded?: boolean;
  children?: FsNode[];
}

function truncationCopy(hits: { results: { path: string }[]; truncation?: 'result' | 'scan' | 'depth' | null }): string {
  const why = hits.truncation === 'scan' ? 'the scan limit' : hits.truncation === 'depth' ? 'the depth limit' : 'the result limit';
  if (!hits.results.length) return `Search stopped at ${why}. This is not a complete list of matches.`;
  return `Showing first ${hits.results.length} matches — stopped at ${why}. Refine your search.`;
}

/** Right-docked, lazily expanding workspace file tree with a name filter. */
export function WorkspaceTree({ root, hidden, onOpenFile, onChooseWorkspace, onError }: {
  root: string;
  hidden: boolean;
  onOpenFile: (path: string) => void;
  onChooseWorkspace: () => void;
  onError: (message: string) => void;
}) {
  const [nodes, setNodes] = useState<FsNode[]>([]);
  const [filter, setFilter] = useState('');
  const [loading, setLoading] = useState(false);
  const [hits, setHits] = useState<{ results: { path: string; kind: string }[]; truncated: boolean; truncation?: 'result' | 'scan' | 'depth' | null } | null>(null);
  const [searching, setSearching] = useState(false);
  const [searchFailed, setSearchFailed] = useState(false);
  const needle = filter.trim();
  const rootRef = useRef(root);
  rootRef.current = root;
  const dirGen = useRef(0);

  const listDir = useCallback(async (path: string, issuedRoot: string): Promise<FsNode[]> => {
    const data = await api.workspaceInspect(path, true, issuedRoot);
    return (data.entries ?? []).map(entry => ({
      name: entry.name,
      kind: entry.kind,
      path: path === '.' ? entry.name : `${path}/${entry.name}`,
      expanded: false,
    }));
  }, []);

  useEffect(() => {
    const generation = ++dirGen.current;
    const issuedRoot = root;
    setNodes([]);
    setHits(null);
    setSearchFailed(false);
    setSearching(false);
    if (!issuedRoot) { setLoading(false); return; }
    setLoading(true);
    listDir('.', issuedRoot)
      .then(next => { if (dirGen.current === generation && rootRef.current === issuedRoot) setNodes(next); })
      .catch(e => { if (dirGen.current === generation && rootRef.current === issuedRoot) onError(errorMessage(e)); })
      .finally(() => { if (dirGen.current === generation && rootRef.current === issuedRoot) setLoading(false); });
  }, [root, listDir, onError]);

  function patch(path: string, update: (node: FsNode) => FsNode) {
    const apply = (list: FsNode[]): FsNode[] =>
      list.map(n => n.path === path ? update(n) : n.children ? { ...n, children: apply(n.children) } : n);
    setNodes(current => apply(current));
  }

  async function toggle(node: FsNode) {
    if (node.kind === 'file') { onOpenFile(node.path); return; }
    if (node.kind !== 'directory') return;
    if (node.loaded) {
      patch(node.path, n => ({ ...n, expanded: !n.expanded }));
      return;
    }
    const issuedRoot = root;
    const generation = dirGen.current;
    try {
      const children = await listDir(node.path, issuedRoot);
      if (rootRef.current !== issuedRoot || dirGen.current !== generation) return;
      patch(node.path, n => ({ ...n, expanded: true, loaded: true, children }));
    } catch (e) {
      if (rootRef.current === issuedRoot) onError(errorMessage(e));
    }
  }

  // Debounced whole-workspace search. The root is part of the request, so the
  // same text starts a new search when the project changes. A late response
  // from the previous root is dropped.
  useEffect(() => {
    if (!needle || !root) {
      setHits(null);
      setSearching(false);
      setSearchFailed(false);
      return;
    }
    let cancelled = false;
    const issuedRoot = root;
    setHits(null);
    setSearchFailed(false);
    setSearching(true);
    const timer = setTimeout(() => {
      api.workspaceSearch(needle, issuedRoot)
        .then(data => { if (!cancelled && rootRef.current === issuedRoot) setHits(data); })
        .catch(e => { if (!cancelled && rootRef.current === issuedRoot) { setSearchFailed(true); onError(errorMessage(e)); } })
        .finally(() => { if (!cancelled && rootRef.current === issuedRoot) setSearching(false); });
    }, 250);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [needle, root, onError]);

  function renderRows(list: FsNode[], depth: number) {
    return list.map(node => (
      <div key={node.path}>
        <button
          className="tree-row"
          style={{ paddingLeft: 6 + depth * 14 }}
          onClick={() => void toggle(node)}
          disabled={node.kind === 'link'}
          title={node.path}
        >
          {node.kind === 'directory'
            ? <ChevronRight size={13} className={`tree-chevron ${node.expanded ? 'open' : ''}`} />
            : <span className="tree-chevron" />}
          {node.kind === 'directory' ? <Folder size={14} /> : <File size={14} />}
          <span className="tree-name">{node.name}</span>
        </button>
        {node.expanded && node.children && renderRows(node.children, depth + 1)}
      </div>
    ));
  }

  return <div className="workspace-tree" hidden={hidden}>
    <div className="tree-toolbar">
      <Search size={14} className="tree-search-icon" />
      <input aria-label="Filter files" placeholder="Filter files…" value={filter} onChange={e => setFilter(e.target.value)} disabled={!root} />
      <button className="icon-button" aria-label="Open workspace folder" disabled={!root} onClick={() => void openPath(root).catch(e => onError(errorMessage(e)))}><FolderOpen size={15} /></button>
    </div>
    <div className="tree-scroll">
      {!root && <div className="tree-empty">
        <FolderSearch size={22} />
        <p>Choose a workspace folder to browse files.</p>
        <button className="project-chip" onClick={onChooseWorkspace}><Folder size={14} />Choose workspace</button>
      </div>}
      {root && !needle && renderRows(nodes, 0)}
      {root && !needle && loading && !nodes.length && <p className="tree-loading">Loading…</p>}
      {root && needle && searching && <p className="tree-loading">Searching…</p>}
      {root && needle && !searching && searchFailed && <p className="tree-loading">Search failed.</p>}
      {root && needle && hits && hits.results.map(result => (
        <button key={result.path} className="tree-row" style={{ paddingLeft: 6 }} title={result.path} onClick={() => onOpenFile(result.path)}>
          <span className="tree-chevron" />
          <File size={14} />
          <span className="tree-name">{result.path}</span>
        </button>
      ))}
      {root && needle && hits && hits.truncated && <p className="tree-loading">{truncationCopy(hits)}</p>}
      {root && needle && hits && !hits.results.length && !hits.truncated && <p className="tree-loading">No matching files.</p>}
    </div>
  </div>;
}

/** Read-only file viewer: breadcrumb, actions, line numbers, highlighting. */
export function FileView({ root, path, onError }: { root: string; path: string; onError: (message: string) => void }) {
  const [content, setContent] = useState<string | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let cancelled = false;
    setContent(null);
    setTruncated(false);
    setFailed(null);
    api.workspaceInspect(path, false, root)
      .then(result => {
        if (cancelled) return;
        setContent((result.content ?? '').replace(/\r\n/g, '\n').replace(/^\d+: /gm, ''));
        setTruncated((result.totalLines ?? 0) > 500);
      })
      .catch(e => {
        if (cancelled) return;
        const message = errorMessage(e);
        setFailed(message);
        onError(message);
      });
    return () => { cancelled = true; };
  }, [root, path, onError]);

  const workspace = root.split(/[\\/]/).filter(Boolean).pop() ?? root;
  const extension = path.split('.').pop()?.toLowerCase() ?? '';
  const rendered = useMemo(() => {
    if (content === null) return null;
    const language = hljs.getLanguage(extension) ? extension : 'plaintext';
    const html = hljs.highlight(content, { language, ignoreIllegals: true }).value;
    return { html, lines: content === '' ? 1 : content.split('\n').length };
  }, [content, extension]);

  return <div className="file-view">
    <div className="file-bar">
      <span className="file-breadcrumb"><strong>{workspace}</strong>{path.split('/').map((part, i) => <span key={i}> › {part}</span>)}</span>
      <div className="file-bar-actions">
        <button className="icon-button" aria-label="Copy file contents" disabled={content === null} onClick={() => {
          void navigator.clipboard.writeText(content ?? '').then(() => { setCopied(true); setTimeout(() => setCopied(false), 1200); }).catch(() => {});
        }}><Copy size={15} />{copied ? ' Copied' : ''}</button>
        <button className="file-open" disabled={!root} onClick={() => void openPath(`${root}/${path}`).catch(e => onError(errorMessage(e)))}><FolderOpen size={14} /> Open</button>
      </div>
    </div>
    <div className="file-code">
      {failed !== null ? <p className="tree-loading" role="alert">Could not open file: {failed}</p>
        : rendered === null ? <p className="tree-loading">Loading…</p> : <pre>
        <span className="file-gutter" aria-hidden="true">{Array.from({ length: rendered.lines }, (_, i) => `${i + 1}\n`).join('')}</span>
        <code dangerouslySetInnerHTML={{ __html: rendered.html }} />
      </pre>}
      {truncated && <p className="tree-loading">Preview truncated at 500 lines.</p>}
    </div>
  </div>;
}
