import { useState, useEffect, useRef } from 'react';
import {
  Archive,
  ChevronDown,
  Folder,
  MessageCircle,
  MoreHorizontal,
  PenSquare,
  Pin,
  Plus,
  Search,
  Trash2,
} from 'lucide-react';
import { confirm, open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { allProjects, conversationProjectId, groupByProject } from '../lib/projects';
import type { Conversation, Hit, Project, WorkspaceIndex } from '../lib/types';

export type Page = 'chat' | 'general' | 'appearance' | 'models' | 'connectors' | 'skills' | 'execution' | 'bash' | 'python' | 'development-tools' | 'language-servers' | 'weather' | 'web-search' | 'inference' | 'diagnostics' | 'security' | 'git' | 'browser-use' | 'computer-use' | 'tools' | 'hooks' | 'permission-rules' | 'plugins';

interface Props {
  page: Page;
  onPage: (page: Page) => void;
  conversations: Conversation[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onNew: (projectId?: string | null) => void;
  busy: boolean;
  search: string;
  onSearch: (search: string) => void;
  theme: string;
  onTheme: () => void;
  workspace?: WorkspaceIndex;
  onWorkspace?: (index: WorkspaceIndex) => void;
  onProject?: (id: string | null) => void;
  onDeleteConversations?: (ids: string[]) => void;
  projectId?: string | null;
  onHit?: (id: string, messageId: string) => void;
}

function formatRelativeTime(timestamp: number, fallback = '2d'): string {
  if (!timestamp || timestamp <= 0) return fallback;
  const diff = Math.max(0, Date.now() - timestamp);
  const minutes = Math.floor(diff / 60000);
  if (minutes < 60) return `${Math.max(1, minutes)}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d`;
  const months = Math.floor(days / 30);
  return `${months}mo`;
}

function readSidebarWidth(): number {
  try {
    return Math.max(220, Math.min(440, Number(localStorage.getItem('locallm-sidebar-width')) || 260));
  } catch {
    return 260;
  }
}

function newProjectId(): string {
  try {
    if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
      return crypto.randomUUID();
    }
  } catch {
    // Fall through to the entropy fallback below.
  }
  return `project-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 12)}`;
}

export function Sidebar(props: Props) {
  const [width, setWidth] = useState(readSidebarWidth);
  useEffect(() => {
    document.documentElement.style.setProperty('--sidebar-width', `${width}px`);
    try {
      localStorage.setItem('locallm-sidebar-width', String(width));
    } catch {
      // Storage may be full or denied; the in-memory width keeps the sidebar usable.
    }
  }, [width]);

  const [searchOpen, setSearchOpen] = useState(false);
  useEffect(() => {
    const showSearch = () => {
      setSearchOpen(true);
      setTimeout(() => document.querySelector<HTMLInputElement>('input[aria-label="Search conversations"]')?.focus(), 0);
    };
    window.addEventListener('locallm-search', showSearch);
    return () => window.removeEventListener('locallm-search', showSearch);
  }, []);

  const [collapsedFolders, setCollapsedFolders] = useState<Record<string, boolean>>({});
  const [expandedProjects, setExpandedProjects] = useState<Record<string, boolean>>({});
  const [unfiledExpanded, setUnfiledExpanded] = useState(false);
  const [menuOpenId, setMenuOpenId] = useState<string | null>(null);
  const [projectMenuOpenId, setProjectMenuOpenId] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const projectMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handlePointerDown(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpenId(null);
      }
      if (projectMenuRef.current && !projectMenuRef.current.contains(e.target as Node)) {
        setProjectMenuOpenId(null);
      }
    }
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        setMenuOpenId(null);
        setProjectMenuOpenId(null);
      }
    }
    window.addEventListener('mousedown', handlePointerDown);
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('mousedown', handlePointerDown);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, []);

  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const workspace = props.workspace ?? { projects: [], tasks: {} };

  async function change(action: () => Promise<WorkspaceIndex>): Promise<WorkspaceIndex | null> {
    setSaving(true);
    setError('');
    try {
      const next = await action();
      props.onWorkspace?.(next);
      return next;
    } catch (e) {
      setError(errorMessage(e));
      return null;
    } finally {
      setSaving(false);
    }
  }

  async function addProject(): Promise<string | null> {
    if (!nativeAvailable) return null;
    try {
      const selected = await open({ directory: true, multiple: false, title: 'Select Project Folder' });
      if (!selected || typeof selected !== 'string') return null;
      const existing = workspace.projects.find(p => p.path === selected);
      if (existing) return existing.id;
      const parts = selected.replace(/\\/g, '/').split('/');
      const name = parts[parts.length - 1] || 'New Project';
      const id = newProjectId();
      const next = await change(() => api.saveProject({ id, name, path: selected }));
      if (!next) return null;
      const saved = next.projects.find(p => p.id === id) ?? next.projects.find(p => p.path === selected);
      return saved?.id ?? null;
    } catch (e) {
      setError(errorMessage(e));
      return null;
    }
  }

  async function handleDeleteProject(project: Project) {
    if (saving || props.busy) return;
    const chatIds = props.conversations
      .filter(c => conversationProjectId(workspace, c.id) === project.id)
      .map(c => c.id);
    const confirmMessage = `Remove "${project.name}" from the project section?\n\nYour files on disk will not be deleted.${chatIds.length ? `\n\nThe ${chatIds.length} chat${chatIds.length === 1 ? '' : 's'} inside will be permanently deleted.` : ''}`;
    const confirmed = nativeAvailable
      ? await confirm(confirmMessage, { title: 'Remove project folder?', kind: 'warning' })
      : window.confirm(confirmMessage);
    if (!confirmed) return;

    if (workspace.projects.some(rp => rp.id === project.id)) {
      await change(async () => {
        const updated = await api.removeProject(project.id);
        props.onDeleteConversations?.(chatIds);
        if (props.projectId === project.id) {
          props.onProject?.(null);
        }
        return updated;
      });
    } else if (props.projectId === project.id) {
      props.onProject?.(null);
    }
  }

  const toggleFolder = (projectId: string) => {
    setCollapsedFolders(prev => ({ ...prev, [projectId]: !prev[projectId] }));
  };

  const [hits, setHits] = useState<Hit[]>([]);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState('');
  const [searchAttempt, setSearchAttempt] = useState(0);

  useEffect(() => {
    const query = props.search.trim();
    setSearchError('');
    if (!query) {
      setHits([]);
      setSearching(false);
      return;
    }
    let cancelled = false;
    setSearching(true);
    const timer = setTimeout(async () => {
      try {
        if (api.searchSessions) {
          const results = await api.searchSessions(query, 20);
          if (!cancelled) setHits(results || []);
        }
      } catch {
        if (!cancelled) { setHits([]); setSearchError('Conversation search failed. Try again.'); }
      } finally {
        if (!cancelled) setSearching(false);
      }
    }, 200);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [props.search, searchAttempt]);

  const queryLower = props.search.toLowerCase();
  const matchingTitleConversations = props.conversations.filter(item =>
    item.title.toLowerCase().includes(queryLower)
  ).sort((a, b) =>
    Number(workspace.tasks[b.id]?.pinned ?? false) - Number(workspace.tasks[a.id]?.pinned ?? false) || b.updatedAt - a.updatedAt
  );

  const effectiveProjects = allProjects(workspace);
  const conversationsByProject = groupByProject(workspace, matchingTitleConversations);
  const unfiledConversations = conversationsByProject.get(null) ?? [];
  const newChatTarget = props.projectId && effectiveProjects.some(p => p.id === props.projectId)
    ? props.projectId
    : effectiveProjects[0]?.id ?? null;
  const newChatTargetName = effectiveProjects.find(p => p.id === newChatTarget)?.name;

  function handleNewChat() {
    if (newChatTarget) {
      props.onNew(newChatTarget);
      return;
    }
    void addProject().then(id => { if (id) props.onNew(id); });
  }

  const isChatActive = (id: string) => {
    if (props.page !== 'chat') return false;
    return props.activeId === id;
  };

  function renderChatItem(c: Conversation) {
    const meta = workspace.tasks[c.id] ?? { projectId: null, archived: false, pinned: false };
    const active = isChatActive(c.id);
    const time = formatRelativeTime(c.updatedAt, '2d');

    return (
      <div
        key={c.id}
        className={`sidebar-task codex-chat-item ${active ? 'active-conversation active-pill' : ''} ${menuOpenId === c.id ? 'menu-open' : ''}`}
      >
        <button
          className="codex-chat-btn"
          aria-label={c.title}
          disabled={props.busy}
          onClick={() => props.onSelect(c.id)}
          title={c.title}
        >
          <span className="codex-chat-title">
            {meta.pinned && <Pin size={10} className="chat-pin-icon" aria-hidden="true" />}
            {c.title}
          </span>
        </button>
        <div className="codex-chat-trailing">
          <span className="codex-chat-time">{time}</span>
          <button
            className="icon-button codex-chat-more-btn"
            aria-label={`Options for ${c.title}`}
            title="Options"
            disabled={props.busy}
            onClick={e => {
              e.stopPropagation();
              setMenuOpenId(menuOpenId === c.id ? null : c.id);
            }}
          >
            <MoreHorizontal size={14} />
          </button>
        </div>
        {menuOpenId === c.id && (
          <div className="task-dropdown-menu" role="menu" ref={menuRef}>
            <button
              aria-label={`${meta.pinned ? 'Unpin' : 'Pin'} ${c.title}`}
              disabled={saving || props.busy || !nativeAvailable}
              onClick={() => {
                void change(() => api.saveTaskMeta(c.id, { ...meta, pinned: !meta.pinned }));
                setMenuOpenId(null);
              }}
            >
              <Pin size={13} />
              <span>{meta.pinned ? 'Unpin task' : 'Pin task'}</span>
            </button>
            <button
              aria-label={`${meta.archived ? 'Restore' : 'Archive'} ${c.title}`}
              disabled={saving || props.busy || !nativeAvailable}
              onClick={() => {
                void change(() => api.saveTaskMeta(c.id, { ...meta, archived: !meta.archived }));
                setMenuOpenId(null);
              }}
            >
              <Archive size={13} />
              <span>{meta.archived ? 'Restore task' : 'Archive task'}</span>
            </button>
          </div>
        )}
      </div>
    );
  }

  return (
    <aside className="sidebar codex-sidebar">
      <div
        className="sidebar-resize"
        role="separator"
        aria-label="Resize sidebar"
        aria-orientation="vertical"
        aria-valuenow={width}
        aria-valuemin={220}
        aria-valuemax={440}
        tabIndex={0}
        onPointerDown={e => e.currentTarget.setPointerCapture(e.pointerId)}
        onPointerMove={e => {
          if (e.currentTarget.hasPointerCapture(e.pointerId)) setWidth(Math.max(220, Math.min(440, e.clientX - (e.currentTarget.parentElement?.getBoundingClientRect().left ?? 0))));
        }}
        onPointerUp={e => e.currentTarget.releasePointerCapture(e.pointerId)}
        onKeyDown={e => {
          if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
            e.preventDefault();
            setWidth(w => Math.max(220, Math.min(440, w + (e.key === 'ArrowLeft' ? -10 : 10))));
          }
        }}
      />

      {/* Home navigation & top actions */}
      <div className="sidebar-top-bar">
        <div className="sidebar-brand-wrapper">
          <button
            className="sidebar-brand-btn"
            aria-label="localLM home"
            title="Home"
            disabled={props.busy}
            onClick={() => props.onPage('chat')}
          >
            <div className="sidebar-brand-mark" aria-hidden="true">
              <svg viewBox="0 0 512 512" width="14" height="14">
                <rect width="512" height="512" rx="112" fill="#191b22" />
                <path d="M154 126h64v230h146v58H154z" fill="#eceef2" />
                <circle cx="339" cy="155" r="42" fill="#3ec98d" />
              </svg>
            </div>
            <span className="sidebar-brand-title">localLM</span>
          </button>
        </div>
        <div className="sidebar-top-actions">
          <button
            className="icon-button"
            aria-label="Search conversations"
            title="Search (Ctrl+K)"
            onClick={() => setSearchOpen(s => !s)}
          >
            <Search size={15} />
          </button>
        </div>
      </div>

      {/* Search Input field */}
      {(searchOpen || props.search.trim()) && (
        <label className="search-field sidebar-search">
          <Search size={14} />
          <input
            type="search"
            autoFocus
            aria-label="Search conversations"
            placeholder="Find a conversation or message…"
            value={props.search}
            onChange={e => props.onSearch(e.target.value)}
          />
        </label>
      )}

      {/* New chat action */}
      <button
        className="sidebar-action-item new-chat"
        aria-label="New conversation"
        title={newChatTargetName ? `New chat in ${newChatTargetName}` : 'New chat — add a project folder first'}
        disabled={props.busy}
        onClick={handleNewChat}
      >
        <PenSquare size={15} className="new-chat-icon" />
        <span>New chat</span>
        <Plus size={14} className="new-chat-plus" />
      </button>



      {/* Projects section heading with add project button */}
      <div className="sidebar-section-header">
        <span>Projects</span>
        <button
          className="add-project-btn"
          aria-label="Add project"
          title="Add project folder"
          disabled={props.busy || saving || !nativeAvailable}
          onClick={() => void addProject()}
        >
          <Plus size={13} />
        </button>
      </div>

      {error && <p role="alert" className="sidebar-error">{error}</p>}

      {/* Project list & nested chats */}
      <div className="conversation-list codex-project-list">
        {!props.search.trim() && !effectiveProjects.length && !unfiledConversations.length && <p className="sidebar-project-empty">Add a folder to start a project.</p>}
        {props.search.trim() ? (
          <>
            <div className="history-heading" role="status" aria-live="polite" aria-atomic="true">
              <span>{searching ? 'Searching conversations…' : 'Search results'}</span>
              <span>{matchingTitleConversations.length + hits.length}</span>
            </div>
            {matchingTitleConversations.map(c => renderChatItem(c))}
            {hits.length > 0 && (
              <div className="search-hits-group">
                <div className="search-hits-heading">
                  <small>Matching messages</small>
                </div>
                {hits.map(hit => (
                  <button
                    key={`${hit.conversationId}-${hit.messageId}`}
                    className={`search-hit-item ${props.activeId === hit.conversationId && props.page === 'chat' ? 'active-conversation' : ''}`}
                    disabled={props.busy}
                    onClick={() => props.onHit ? props.onHit(hit.conversationId, hit.messageId) : props.onSelect(hit.conversationId)}
                    title={`${hit.conversationTitle}: ${hit.excerpt}`}
                  >
                    <div className="hit-title">
                      <MessageCircle size={12} />
                      <strong>{hit.conversationTitle}</strong>
                    </div>
                    <div className="hit-excerpt">{hit.excerpt}</div>
                  </button>
                ))}
              </div>
            )}
            {searchError && <p className="error" role="alert">{searchError}<button onClick={() => setSearchAttempt(attempt => attempt + 1)}>Retry search</button></p>}
            {!searchError && !matchingTitleConversations.length && !hits.length && !searching && (
              <p className="history-empty">No matching conversations found.</p>
            )}
          </>
        ) : (
          <>
            {effectiveProjects.map(project => {
              const projectConversations = conversationsByProject.get(project.id) ?? [];

              const isCollapsed = Boolean(collapsedFolders[project.id]);
              const isExpanded = Boolean(expandedProjects[project.id]);
              const displayLimit = isExpanded ? projectConversations.length : 6;
              const displayConversations = projectConversations.slice(0, displayLimit);
              const totalCount = projectConversations.length;
              const hasMore = projectConversations.length > 6;

              return (
                <div className="codex-project-group" key={project.id}>
                  <div
                    className={`codex-folder-header ${props.projectId === project.id ? 'active-folder' : ''} ${projectMenuOpenId === project.id ? 'menu-open' : ''}`}
                    onContextMenu={e => {
                      e.preventDefault();
                      setProjectMenuOpenId(project.id);
                    }}
                  >
                    <button
                      type="button"
                      className="codex-folder-label"
                      aria-expanded={!isCollapsed}
                      title={project.path ? `${project.name} (${project.path})` : project.name}
                      onClick={() => toggleFolder(project.id)}
                    >
                      <ChevronDown
                        size={13}
                        className={`codex-folder-chevron ${isCollapsed ? 'collapsed' : ''}`}
                      />
                      <Folder size={14} className="codex-folder-icon" />
                      <span className="codex-folder-name">{project.name}</span>
                    </button>

                    {totalCount > 0 && (
                      <span className="codex-folder-count" aria-hidden="true">{totalCount}</span>
                    )}

                    <div className="codex-folder-actions" onClick={e => e.stopPropagation()}>
                      <button
                        className="codex-folder-action-btn"
                        aria-label={`New chat in ${project.name}`}
                        title={`New chat in ${project.name}`}
                        disabled={props.busy}
                        onClick={e => {
                          e.stopPropagation();
                          setCollapsedFolders(prev => ({ ...prev, [project.id]: false }));
                          props.onNew(project.id);
                        }}
                      >
                        <Plus size={13} />
                      </button>

                      <button
                        className="codex-folder-action-btn action-danger"
                        aria-label={`Delete folder ${project.name}`}
                        title={`Delete folder ${project.name}`}
                        disabled={props.busy || saving}
                        onClick={e => {
                          e.stopPropagation();
                          void handleDeleteProject(project);
                        }}
                      >
                        <Trash2 size={13} />
                      </button>

                      <button
                        className="codex-folder-action-btn codex-folder-more-btn"
                        aria-label={`Options for ${project.name}`}
                        title="Options"
                        disabled={props.busy || saving}
                        onClick={e => {
                          e.stopPropagation();
                          setProjectMenuOpenId(projectMenuOpenId === project.id ? null : project.id);
                        }}
                      >
                        <MoreHorizontal size={13} />
                      </button>

                      {projectMenuOpenId === project.id && (
                        <div
                          className="task-dropdown-menu codex-folder-dropdown-menu"
                          role="menu"
                          ref={projectMenuRef}
                          onClick={e => e.stopPropagation()}
                        >
                          <button
                            role="menuitem"
                            aria-label={`New chat in ${project.name}`}
                            disabled={props.busy}
                            onClick={() => {
                              setProjectMenuOpenId(null);
                              setCollapsedFolders(prev => ({ ...prev, [project.id]: false }));
                              props.onNew(project.id);
                            }}
                          >
                            <Plus size={13} />
                            <span>New chat</span>
                          </button>
                          <button
                            role="menuitem"
                            className="menu-item-danger"
                            aria-label={`Delete folder ${project.name}`}
                            disabled={saving || props.busy}
                            onClick={() => {
                              setProjectMenuOpenId(null);
                              void handleDeleteProject(project);
                            }}
                          >
                            <Trash2 size={13} />
                            <span>Delete folder</span>
                          </button>
                        </div>
                      )}
                    </div>
                  </div>

                  {!isCollapsed && (
                    <div className="codex-nested-tasks">
                      {totalCount === 0 && (
                        <p className="codex-folder-empty">No chats yet</p>
                      )}

                      {displayConversations.map(c => renderChatItem(c))}

                      {hasMore && (
                        <button
                          className="codex-see-all-btn"
                          onClick={() => setExpandedProjects(p => ({ ...p, [project.id]: !p[project.id] }))}
                        >
                          {isExpanded ? 'Show less' : `See all (${totalCount})`}
                        </button>
                      )}
                    </div>
                  )}
                </div>
              );
            })}

            {unfiledConversations.length > 0 && (
              <div className="codex-project-group">
                {(unfiledExpanded ? unfiledConversations : unfiledConversations.slice(0, 6)).map(c => renderChatItem(c))}
                {unfiledConversations.length > 6 && (
                  <button
                    className="codex-see-all-btn"
                    onClick={() => setUnfiledExpanded(v => !v)}
                  >
                    {unfiledExpanded ? 'Show less' : `See all (${unfiledConversations.length})`}
                  </button>
                )}
              </div>
            )}

            <div className="codex-project-list-footer">
              <button
                className="codex-empty-add-btn"
                onClick={() => void addProject()}
                disabled={props.busy || saving || !nativeAvailable}
              >
                <Plus size={12} />
                <span>Add project folder</span>
              </button>
            </div>
          </>
        )}
      </div>
    </aside>
  );
}
