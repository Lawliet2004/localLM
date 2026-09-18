import { useState, useEffect, useRef } from 'react';
import {
  Archive,
  AtSign,
  Bell,
  BookOpen,
  ChevronDown,
  Clock,
  Cpu,
  Folder,
  MessageCircle,
  MessageSquare,
  MoreHorizontal,
  PenSquare,
  Pin,
  Plug,
  Plus,
  Search,
  Settings,
  Terminal,
  Trash2,
  Wrench,
} from 'lucide-react';
import { confirm, open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { Conversation, Hit, Project, WorkspaceIndex } from '../lib/types';

export type Page = 'chat' | 'models' | 'connectors' | 'skills' | 'execution' | 'tools' | 'automations' | 'plugins';

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
  onCollapse: () => void;
  workspace?: WorkspaceIndex;
  onWorkspace?: (index: WorkspaceIndex) => void;
  onProject?: (id: string | null) => void;
  projectId?: string | null;
  onHit?: (id: string, messageId: string) => void;
}

const DEFAULT_PROJECTS = [
  { id: 'locallm', name: 'localLM', path: '' },
];

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

export function Sidebar(props: Props) {
  const [width, setWidth] = useState(() => Math.max(220, Math.min(440, Number(localStorage.getItem('locallm-sidebar-width')) || 260)));
  useEffect(() => {
    document.documentElement.style.setProperty('--sidebar-width', `${width}px`);
    localStorage.setItem('locallm-sidebar-width', String(width));
  }, [width]);

  const [brandMenuOpen, setBrandMenuOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [collapsedFolders, setCollapsedFolders] = useState<Record<string, boolean>>({});
  const [expandedProjects, setExpandedProjects] = useState<Record<string, boolean>>({});
  const [menuOpenId, setMenuOpenId] = useState<string | null>(null);
  const [projectMenuOpenId, setProjectMenuOpenId] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const projectMenuRef = useRef<HTMLDivElement>(null);
  const brandRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handlePointerDown(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setMenuOpenId(null);
      }
      if (projectMenuRef.current && !projectMenuRef.current.contains(e.target as Node)) {
        setProjectMenuOpenId(null);
      }
      if (brandRef.current && !brandRef.current.contains(e.target as Node)) {
        setBrandMenuOpen(false);
      }
    }
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        setMenuOpenId(null);
        setProjectMenuOpenId(null);
        setBrandMenuOpen(false);
      }
    }
    window.addEventListener('mousedown', handlePointerDown);
    window.addEventListener('keydown', handleKeyDown);
    return () => {
      window.removeEventListener('mousedown', handlePointerDown);
      window.removeEventListener('keydown', handleKeyDown);
    };
  }, []);

  const [deletedDefaultProjects, setDeletedDefaultProjects] = useState<string[]>(() => {
    try {
      const stored = localStorage.getItem('locallm-deleted-default-projects');
      return stored ? JSON.parse(stored) : [];
    } catch {
      return [];
    }
  });

  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const workspace = props.workspace ?? { projects: [], tasks: {} };

  async function change(action: () => Promise<WorkspaceIndex>) {
    setSaving(true);
    setError('');
    try {
      props.onWorkspace?.(await action());
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  async function addProject() {
    if (!nativeAvailable) return;
    try {
      const selected = await open({ directory: true, multiple: false, title: 'Select Project Folder' });
      if (selected && typeof selected === 'string') {
        const parts = selected.replace(/\\/g, '/').split('/');
        const name = parts[parts.length - 1] || 'New Project';
        const id = name.toLowerCase().replace(/[^a-z0-9]/g, '-');
        if (deletedDefaultProjects.includes(id)) {
          setDeletedDefaultProjects(prev => {
            const next = prev.filter(p => p !== id);
            try {
              localStorage.setItem('locallm-deleted-default-projects', JSON.stringify(next));
            } catch {
              // ignore
            }
            return next;
          });
        }
        await change(() => api.saveProject({ id, name, path: selected }));
      }
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function handleDeleteProject(project: Project) {
    if (saving || props.busy) return;
    const confirmMessage = `Remove "${project.name}" from the project section?\n\nYour files on disk will not be deleted. Any chats in this folder will remain in general chats.`;
    const confirmed = nativeAvailable
      ? await confirm(confirmMessage, { title: 'Remove project folder?', kind: 'warning' })
      : window.confirm(confirmMessage);
    if (!confirmed) return;

    if (DEFAULT_PROJECTS.some(dp => dp.id === project.id)) {
      setDeletedDefaultProjects(prev => {
        const next = Array.from(new Set([...prev, project.id]));
        try {
          localStorage.setItem('locallm-deleted-default-projects', JSON.stringify(next));
        } catch {
          // ignore
        }
        return next;
      });
    }

    if (workspace.projects.some(rp => rp.id === project.id)) {
      await change(async () => {
        const updated = await api.removeProject(project.id);
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

  useEffect(() => {
    const query = props.search.trim();
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
        if (!cancelled) setHits([]);
      } finally {
        if (!cancelled) setSearching(false);
      }
    }, 200);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [props.search]);

  const queryLower = props.search.toLowerCase();
  const matchingTitleConversations = props.conversations.filter(item =>
    item.title.toLowerCase().includes(queryLower)
  ).sort((a, b) =>
    Number(workspace.tasks[b.id]?.pinned ?? false) - Number(workspace.tasks[a.id]?.pinned ?? false) || b.updatedAt - a.updatedAt
  );

  const filteredDefaultProjects = DEFAULT_PROJECTS.filter(dp => !deletedDefaultProjects.includes(dp.id));
  const registeredProjects = workspace.projects.length > 0 ? workspace.projects : [];
  const hasUnassigned = matchingTitleConversations.some(c => {
    const pId = workspace.tasks[c.id]?.projectId;
    return !pId || !registeredProjects.some(rp => rp.id === pId);
  });
  const showDefaultProject = (registeredProjects.length === 0 || hasUnassigned) && filteredDefaultProjects.length > 0;
  const effectiveProjects = [
    ...registeredProjects,
    ...(showDefaultProject ? filteredDefaultProjects.filter(dp => !registeredProjects.some(rp => rp.id.toLowerCase() === dp.id.toLowerCase() || rp.name.toLowerCase() === dp.name.toLowerCase())) : []),
  ];

  const isLocallmActive = effectiveProjects.some(p => p.id === 'locallm');
  const unassignedConversations = matchingTitleConversations.filter(c => {
    const taskMeta = workspace.tasks[c.id];
    return !taskMeta?.projectId || !effectiveProjects.some(p => p.id === taskMeta.projectId);
  });

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
            {meta.pinned && <span className="pinned-indicator">⌁ </span>}
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
          <div className="codex-task-dropdown-menu" role="menu" ref={menuRef}>
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
          if (e.currentTarget.hasPointerCapture(e.pointerId)) setWidth(Math.max(220, Math.min(440, e.clientX)));
        }}
        onPointerUp={e => e.currentTarget.releasePointerCapture(e.pointerId)}
        onKeyDown={e => {
          if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
            e.preventDefault();
            setWidth(w => Math.max(220, Math.min(440, w + (e.key === 'ArrowLeft' ? -10 : 10))));
          }
        }}
      />

      {/* Top Header: Brand Dropdown & Top Actions */}
      <div className="sidebar-top-bar">
        <div className="sidebar-brand-wrapper" ref={brandRef}>
          <button
            className="sidebar-brand-btn"
            aria-label="Workspace menu"
            aria-expanded={brandMenuOpen}
            onClick={() => setBrandMenuOpen(b => !b)}
          >
            <div className="sidebar-brand-mark" aria-hidden="true">
              <svg viewBox="0 0 512 512" width="14" height="14">
                <rect width="512" height="512" rx="112" fill="#1c1d1f" />
                <path d="M154 126h64v230h146v58H154z" fill="#e9e9e5" />
                <circle cx="339" cy="155" r="42" fill="#10b981" />
              </svg>
            </div>
            <span className="sidebar-brand-title">localLM</span>
            <ChevronDown size={14} className={`sidebar-brand-chevron ${brandMenuOpen ? 'open' : ''}`} />
          </button>
          {brandMenuOpen && (
            <div className="sidebar-brand-menu" role="menu">
              <button
                role="menuitem"
                onClick={() => {
                  props.onPage('chat');
                  setBrandMenuOpen(false);
                }}
              >
                LocalLM Workspace
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  props.onPage('connectors');
                  setBrandMenuOpen(false);
                }}
              >
                Connectors
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  props.onPage('skills');
                  setBrandMenuOpen(false);
                }}
              >
                Skills
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  props.onPage('tools');
                  setBrandMenuOpen(false);
                }}
              >
                Tools &amp; Permissions
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  props.onPage('models');
                  setBrandMenuOpen(false);
                }}
              >
                Models &amp; Runtime
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  props.onPage('execution');
                  setBrandMenuOpen(false);
                }}
              >
                Execution
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  props.onPage('automations');
                  setBrandMenuOpen(false);
                }}
              >
                Scheduled Tasks
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  props.onPage('plugins');
                  setBrandMenuOpen(false);
                }}
              >
                Plugins
              </button>
            </div>
          )}
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
          <button
            className="icon-button"
            aria-label="Notifications"
            title="Notifications"
            onClick={() => {}}
          >
            <Bell size={15} />
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
        disabled={props.busy}
        onClick={() => props.onNew(null)}
      >
        <PenSquare size={15} className="new-chat-icon" />
        <span>New chat</span>
        <Plus size={14} className="new-chat-plus" />
      </button>

      {/* Top quick navigation links: Toolsets & Features */}
      <nav className="codex-nav locallm-nav" aria-label="Quick links">
        <button
          className={props.page === 'connectors' ? 'selected' : ''}
          aria-label="Connectors"
          title="Connectors (MCP servers, external tools)"
          onClick={() => props.onPage('connectors')}
        >
          <Plug size={15} />
          <span>Connectors</span>
        </button>
        <button
          className={props.page === 'skills' ? 'selected' : ''}
          aria-label="Skills"
          title="Skills (Reusable workflow instructions & scripts)"
          onClick={() => props.onPage('skills')}
        >
          <BookOpen size={15} />
          <span>Skills</span>
        </button>
        <button
          className={props.page === 'tools' ? 'selected' : ''}
          aria-label="Tools"
          title="Tools & permissions"
          onClick={() => props.onPage('tools')}
        >
          <Wrench size={15} />
          <span>Tools</span>
        </button>
        <button
          className={props.page === 'models' ? 'selected' : ''}
          aria-label="Models & runtime"
          title="Models & runtime (Local GGUF, llama.cpp, API providers)"
          onClick={() => props.onPage('models')}
        >
          <Cpu size={15} />
          <span>Models &amp; runtime</span>
        </button>
        <button
          className={props.page === 'execution' ? 'selected' : ''}
          aria-label="Execution"
          title="Execution (Interpreters, Web Search)"
          onClick={() => props.onPage('execution')}
        >
          <Terminal size={15} />
          <span>Execution</span>
        </button>
        <button
          className={props.page === 'automations' ? 'selected' : ''}
          aria-label="Scheduled"
          title="Scheduled tasks"
          onClick={() => props.onPage('automations')}
        >
          <Clock size={15} />
          <span>Scheduled</span>
        </button>
        <button
          className={props.page === 'plugins' ? 'selected' : ''}
          aria-label="Plugins"
          title="Plugins"
          onClick={() => props.onPage('plugins')}
        >
          <AtSign size={15} />
          <span>Plugins</span>
        </button>
      </nav>

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
        {props.search.trim() ? (
          <>
            <div className="history-heading">
              <span>{searching ? 'SEARCHING…' : 'SEARCH RESULTS'}</span>
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
            {!matchingTitleConversations.length && !hits.length && !searching && (
              <p className="history-empty">No matching conversations found.</p>
            )}
          </>
        ) : (
          <>
            {effectiveProjects.map(project => {
              const projectConversations = matchingTitleConversations.filter(c => {
                const taskMeta = workspace.tasks[c.id];
                return taskMeta?.projectId ? taskMeta.projectId === project.id : (project.id === 'locallm');
              });

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
                    onClick={() => toggleFolder(project.id)}
                    onContextMenu={e => {
                      e.preventDefault();
                      setProjectMenuOpenId(project.id);
                    }}
                    title={project.path ? `${project.name} (${project.path})` : project.name}
                  >
                    <div className="codex-folder-label">
                      <ChevronDown
                        size={13}
                        className={`codex-folder-chevron ${isCollapsed ? 'collapsed' : ''}`}
                      />
                      <Folder size={15} className="codex-folder-icon" />
                      <span className="codex-folder-name">{project.name}</span>
                    </div>

                    <div className="codex-folder-actions" onClick={e => e.stopPropagation()}>
                      <button
                        className="codex-folder-action-btn"
                        aria-label={`New chat in ${project.name}`}
                        title={`New chat in ${project.name}`}
                        disabled={props.busy}
                        onClick={e => {
                          e.stopPropagation();
                          setCollapsedFolders(prev => ({ ...prev, [project.id]: false }));
                          props.onNew(project.id === 'locallm' ? null : project.id);
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
                          className="codex-task-dropdown-menu codex-folder-dropdown-menu"
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
                              props.onNew(project.id === 'locallm' ? null : project.id);
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

            {!isLocallmActive && unassignedConversations.length > 0 && (
              <div className="codex-project-group" key="_unassigned">
                <div
                  className="codex-folder-header codex-general-header"
                  onClick={() => toggleFolder('_unassigned')}
                  title="General chats (not assigned to any folder)"
                >
                  <div className="codex-folder-label">
                    <ChevronDown
                      size={13}
                      className={`codex-folder-chevron ${collapsedFolders['_unassigned'] ? 'collapsed' : ''}`}
                    />
                    <MessageSquare size={14} className="codex-folder-icon" />
                    <span className="codex-folder-name">General chats</span>
                  </div>
                  <div className="codex-folder-actions" onClick={e => e.stopPropagation()}>
                    <button
                      className="codex-folder-action-btn"
                      aria-label="New general chat"
                      title="New general chat"
                      disabled={props.busy}
                      onClick={e => {
                        e.stopPropagation();
                        props.onNew(null);
                      }}
                    >
                      <Plus size={13} />
                    </button>
                  </div>
                </div>
                {!collapsedFolders['_unassigned'] && (
                  <div className="codex-nested-tasks">
                    {unassignedConversations.map(c => renderChatItem(c))}
                  </div>
                )}
              </div>
            )}

            {effectiveProjects.length === 0 && unassignedConversations.length === 0 && (
              <div className="codex-empty-folders">
                <p className="codex-empty-folders-text">No project folders</p>
                <button
                  className="codex-empty-add-btn"
                  onClick={() => void addProject()}
                  disabled={props.busy || saving || !nativeAvailable}
                >
                  <Plus size={12} />
                  <span>Add project folder</span>
                </button>
              </div>
            )}
          </>
        )}
      </div>

      <div className="sidebar-footer codex-sidebar-footer">
        <button
          className={`sidebar-settings-btn ${props.page !== 'chat' ? 'active-settings' : ''}`}
          aria-label="Settings"
          onClick={() => props.onPage(props.page === 'chat' ? 'models' : 'chat')}
          title="Settings (Models & runtime, Connectors, Skills, Tools, Execution, Scheduled, Plugins)"
        >
          <Settings size={15} className="settings-gear-icon" />
          <span>Settings</span>
        </button>
      </div>
    </aside>
  );
}
