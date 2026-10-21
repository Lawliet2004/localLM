import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronRight, File, FilePlus, Folder, FolderOpen, FolderPlus, FolderSearch, Pencil, RefreshCw, Search, Trash2 } from 'lucide-react';
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

/** Lazily expanding editor explorer with a workspace-wide name filter. */
export type TreeAction = 'createFile' | 'createDirectory' | 'rename' | 'delete';
export function WorkspaceTree({ root, hidden, onOpenFile, onChooseWorkspace, onError, revision = 0, onAction, disabled = false, selectedPath }: {
  root: string;
  hidden: boolean;
  onOpenFile: (path: string) => void;
  onChooseWorkspace: () => void;
  onError: (message: string) => void;
  revision?: number;
  onAction?: (action: TreeAction, path: string, directory: boolean) => void;
  disabled?: boolean;
  selectedPath?: string;
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
  const [selected, setSelected] = useState<FsNode | null>(null);
  const [refresh, setRefresh] = useState(0);
  const [truncated, setTruncated] = useState(false);
  const expandedPaths = useRef(new Set<string>());
  const loadedRoot = useRef(root);

  const listDir = useCallback(async (path: string, issuedRoot: string): Promise<FsNode[]> => {
    const data = await api.workspaceInspect(path, true, issuedRoot);
    if (path === '.' && rootRef.current === issuedRoot) setTruncated(!!data.truncated);
    return (data.entries ?? []).sort((a, b) => Number(b.kind === 'directory') - Number(a.kind === 'directory') || a.name.localeCompare(b.name)).map(entry => ({
      name: entry.name,
      kind: entry.kind,
      path: path === '.' ? entry.name : `${path}/${entry.name}`,
      expanded: false,
    }));
  }, []);

  useEffect(() => {
    if (hidden) return;
    const generation = ++dirGen.current;
    const issuedRoot = root;
    setNodes([]);
    setHits(null);
    setSearchFailed(false);
    setSearching(false);
    if (loadedRoot.current !== root) { expandedPaths.current.clear(); setSelected(null); loadedRoot.current = root; }
    if (!issuedRoot) { setLoading(false); return; }
    setLoading(true);
    const restore = async (path: string): Promise<FsNode[]> => {
      const children = await listDir(path, issuedRoot);
      return Promise.all(children.map(async node => expandedPaths.current.has(node.path) && node.kind === 'directory'
        ? { ...node, expanded: true, loaded: true, children: await restore(node.path).catch(() => []) } : node));
    };
    restore('.')
      .then(next => { if (dirGen.current === generation && rootRef.current === issuedRoot) setNodes(next); })
      .catch(e => { if (dirGen.current === generation && rootRef.current === issuedRoot) onError(errorMessage(e)); })
      .finally(() => { if (dirGen.current === generation && rootRef.current === issuedRoot) setLoading(false); });
    return () => { ++dirGen.current; };
  }, [root, hidden, listDir, onError, revision, refresh]);

  useEffect(() => {
    setSelected(selectedPath ? { path: selectedPath, name: selectedPath.split('/').pop()!, kind: 'file', expanded: false } : null);
  }, [root, selectedPath, revision]);

  function patch(path: string, update: (node: FsNode) => FsNode) {
    const apply = (list: FsNode[]): FsNode[] =>
      list.map(n => n.path === path ? update(n) : n.children ? { ...n, children: apply(n.children) } : n);
    setNodes(current => apply(current));
  }

  async function toggle(node: FsNode) {
    setSelected(node);
    if (node.kind === 'file') { onOpenFile(node.path); return; }
    if (node.kind !== 'directory') return;
    if (node.loaded) {
      if (node.expanded) expandedPaths.current.delete(node.path); else expandedPaths.current.add(node.path);
      patch(node.path, n => ({ ...n, expanded: !n.expanded }));
      return;
    }
    const issuedRoot = root;
    const generation = dirGen.current;
    try {
      const children = await listDir(node.path, issuedRoot);
      if (rootRef.current !== issuedRoot || dirGen.current !== generation) return;
      expandedPaths.current.add(node.path);
      patch(node.path, n => ({ ...n, expanded: true, loaded: true, children }));
    } catch (e) {
      if (rootRef.current === issuedRoot) onError(errorMessage(e));
    }
  }

  // Debounced whole-workspace search. The root is part of the request, so the
  // same text starts a new search when the project changes. A late response
  // from the previous root is dropped.
  useEffect(() => {
    if (hidden || !needle || !root) {
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
  }, [needle, root, hidden, onError, revision, refresh]);

  const targetFolder = selected?.kind === 'directory' ? selected.path : selected?.path.split('/').slice(0, -1).join('/') ?? '';
  function action(kind: TreeAction) {
    if (kind === 'createFile' || kind === 'createDirectory') {
      if (targetFolder) expandedPaths.current.add(targetFolder);
      onAction?.(kind, targetFolder, true);
    } else if (selected) onAction?.(kind, selected.path, selected.kind === 'directory');
  }

  function renderRows(list: FsNode[], depth: number) {
    return list.map(node => (
      <div key={node.path}>
        <button
          className={`tree-row ${selected?.path === node.path || selectedPath === node.path ? 'selected' : ''}`}
          style={{ paddingLeft: 6 + depth * 14 }}
          onClick={() => void toggle(node)}
          disabled={node.kind === 'link'}
          aria-expanded={node.kind === 'directory' ? node.expanded : undefined}
          title={node.path}
        >
          {node.kind === 'directory'
            ? <ChevronRight size={13} className={`tree-chevron ${node.expanded ? 'open' : ''}`} />
            : <span className="tree-chevron" />}
          {node.kind === 'directory' ? <Folder size={14} /> : <File size={14} />}
          <span className="tree-name">{node.name}</span>
        </button>
        {node.expanded && node.children && (node.children.length ? renderRows(node.children, depth + 1) : <p className="tree-folder-empty" style={{ paddingLeft: 24 + depth * 14 }}>Empty folder</p>)}
      </div>
    ));
  }

  return <div className="workspace-tree" hidden={hidden}>
    <div className="tree-heading">
      <button title={root || 'Choose workspace'} onClick={onChooseWorkspace} disabled={disabled}><FolderOpen size={14} /><span>{root.split(/[\\/]/).filter(Boolean).pop() || 'Explorer'}</span></button>
      <button className="icon-button" aria-label="Refresh explorer" disabled={!root || loading} onClick={() => { setSelected(null); setRefresh(n => n + 1); }}><RefreshCw size={14} /></button>
    </div>
    <div className="tree-toolbar">
      <Search size={14} className="tree-search-icon" />
      <input aria-label="Filter files" placeholder="Filter files…" value={filter} onChange={e => setFilter(e.target.value)} disabled={!root} />
      <button className="icon-button" aria-label="Open workspace folder" disabled={!root} onClick={() => void api.openWorkspacePath(root).catch(e => onError(errorMessage(e)))}><FolderOpen size={15} /></button>
    </div>
    {onAction && <div className="tree-actions" aria-label="File actions">
      <button className="icon-button" aria-label="New file" title={targetFolder ? `New file in ${targetFolder}` : 'New file in workspace'} disabled={!root || disabled} onClick={() => action('createFile')}><FilePlus size={15} /></button>
      <button className="icon-button" aria-label="New folder" title={targetFolder ? `New folder in ${targetFolder}` : 'New folder in workspace'} disabled={!root || disabled} onClick={() => action('createDirectory')}><FolderPlus size={15} /></button>
      <button className="icon-button" aria-label="Rename selected" title="Rename selected file or folder" disabled={!selected || selected.kind === 'link' || disabled} onClick={() => action('rename')}><Pencil size={14} /></button>
      <button className="icon-button" aria-label="Delete selected" title="Delete selected file or folder" disabled={!selected || selected.kind === 'link' || disabled} onClick={() => action('delete')}><Trash2 size={14} /></button>
      {selected && <button className="tree-clear" onClick={() => setSelected(null)} title="Clear selection; create at workspace root">Root</button>}
    </div>}
    <div className="tree-scroll">
      {!root && <div className="tree-empty">
        <FolderSearch size={22} />
        <p>Choose a workspace folder to browse files.</p>
        <button className="project-chip" onClick={onChooseWorkspace}><Folder size={14} />Choose workspace</button>
      </div>}
      {root && !needle && renderRows(nodes, 0)}
      {root && !needle && loading && !nodes.length && <p className="tree-loading">Loading…</p>}
      {root && !needle && !loading && !nodes.length && <p className="tree-loading">This folder is empty. Create a file or folder above.</p>}
      {root && !needle && truncated && <p className="tree-loading">Showing the first 500 entries. Use the filter to find other files.</p>}
      {root && needle && searching && <p className="tree-loading">Searching…</p>}
      {root && needle && !searching && searchFailed && <p className="tree-loading">Search failed.</p>}
      {root && needle && hits && hits.results.map(result => (
        <button key={result.path} className={`tree-row ${selected?.path === result.path ? 'selected' : ''}`} style={{ paddingLeft: 6 }} title={result.path} onClick={() => { setSelected({ name: result.path.split('/').pop()!, path: result.path, kind: 'file', expanded: false }); onOpenFile(result.path); }}>
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

