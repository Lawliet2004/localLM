import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { ChevronDown, ChevronRight, ListTodo, Pause, Play, RotateCcw, Trash2, XCircle } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { Recipe, Task } from '../lib/types';

const STATUS_LABEL: Record<string, string> = {
  queued: 'Queued',
  running: 'Running',
  waiting_approval: 'Needs approval',
  waiting_resource: 'Waiting',
  paused: 'Paused',
  interrupted: 'Interrupted',
  failed: 'Failed',
  completed: 'Done',
  awaiting_review: 'Needs review',
  cancelled: 'Cancelled',
};

const ACTIVE = new Set(['queued', 'running', 'waiting_approval', 'waiting_resource', 'paused', 'interrupted', 'awaiting_review']);

interface Props {
  onSelectConversation: (id: string) => void;
}

export function TaskInbox({ onSelectConversation }: Props) {
  const [open, setOpen] = useState(true);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [error, setError] = useState('');
  const [announcement, setAnnouncement] = useState('');
  const statusesRef = useRef<Map<string, string>>(new Map());

  const refresh = useCallback(async () => {
    if (!nativeAvailable) return;
    try {
      const list = await api.tasksList();
      setTasks(list);
      // Announce transitions for screen readers; keep only the newest.
      const next = new Map<string, string>();
      let changed = '';
      for (const task of list) {
        next.set(task.id, task.status);
        const previous = statusesRef.current.get(task.id);
        if (previous && previous !== task.status) {
          changed = `Task "${task.content.slice(0, 60)}" is now ${STATUS_LABEL[task.status] ?? task.status}.`;
        }
      }
      statusesRef.current = next;
      if (changed) setAnnouncement(changed);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 3000);
    let unlisten: (() => void) | undefined;
    let unlistenEvents: (() => void) | undefined;
    if (nativeAvailable) {
      // Outside Tauri (tests, browser) listen() rejects — keep the poll.
      void listen('tasks-changed', () => void refresh()).then(u => { unlisten = u; }).catch(() => {});
      void listen('task-event', () => void refresh()).then(u => { unlistenEvents = u; }).catch(() => {});
    }
    return () => {
      window.clearInterval(timer);
      unlisten?.();
      unlistenEvents?.();
    };
  }, [refresh]);

  const act = async (action: () => Promise<unknown>) => {
    setError('');
    try {
      await action();
      await refresh();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  if (!nativeAvailable) return null;
  const visible = tasks.filter(task => ACTIVE.has(task.status) || task.status === 'failed' || task.status === 'completed').slice(0, 30);
  if (!open) {
    return (
      <button type="button" className="task-inbox-toggle" onClick={() => setOpen(true)} aria-expanded="false">
        <ChevronRight size={12} aria-hidden="true" />
        <ListTodo size={13} aria-hidden="true" />
        <span>Tasks</span>
        {visible.some(task => ACTIVE.has(task.status)) && <span className="task-inbox-count">{visible.filter(task => ACTIVE.has(task.status)).length}</span>}
      </button>
    );
  }
  return (
    <section className="task-inbox" aria-label="Task inbox">
      <button type="button" className="task-inbox-toggle" onClick={() => setOpen(false)} aria-expanded="true">
        <ChevronDown size={12} aria-hidden="true" />
        <ListTodo size={13} aria-hidden="true" />
        <span>Tasks</span>
      </button>
      <p className="sr-only" aria-live="polite">{announcement}</p>
      {error && <p role="alert" className="sidebar-error">{error}</p>}
      {visible.length === 0 && <p className="task-inbox-empty">No tasks yet. Queue work from the composer.</p>}
      <ul className="task-inbox-list">
        {visible.map(task => {
          const label = STATUS_LABEL[task.status] ?? task.status;
          const busyState = ['running', 'waiting_approval', 'waiting_resource'].includes(task.status);
          return (
            <li key={task.id} className="task-inbox-item">
              <button type="button" className="task-inbox-title" onClick={() => onSelectConversation(task.conversationId)} title={task.content}>
                <span className="task-inbox-text">{task.content.slice(0, 80)}</span>
              </button>
              <div className="task-inbox-meta">
                <span className={`task-chip task-chip-${task.status}`}>{label}</span>
                {task.waitReason && <span className="task-inbox-reason" title={task.waitReason}>{task.waitReason}</span>}
                {task.nextAction && <span className="task-inbox-reason task-inbox-next" title={task.nextAction}>{task.nextAction}</span>}
                {task.error && task.status === 'failed' && <span className="task-inbox-reason" title={task.error}>{task.error}</span>}
              </div>
              <div className="task-inbox-actions">
                {busyState && (
                  <button type="button" className="task-action" title="Pause at the next step boundary" aria-label={`Pause task ${task.content.slice(0, 30)}`} onClick={() => void act(() => api.taskPause(task.id))}>
                    <Pause size={11} aria-hidden="true" />
                  </button>
                )}
                {['paused', 'interrupted'].includes(task.status) && (
                  <button type="button" className="task-action" title="Resume" aria-label={`Resume task ${task.content.slice(0, 30)}`} onClick={() => void act(() => api.taskResume(task.id))}>
                    <Play size={11} aria-hidden="true" />
                  </button>
                )}
                {['failed', 'cancelled', 'interrupted', 'awaiting_review'].includes(task.status) && (
                  <button type="button" className="task-action" title="Retry as a new task" aria-label={`Retry task ${task.content.slice(0, 30)}`} onClick={() => void act(() => api.taskRetry(task.id))}>
                    <RotateCcw size={11} aria-hidden="true" />
                  </button>
                )}
                {ACTIVE.has(task.status) && (
                  <button type="button" className="task-action task-action-danger" title="Cancel" aria-label={`Cancel task ${task.content.slice(0, 30)}`} onClick={() => void act(() => api.taskCancel(task.id))}>
                    <XCircle size={11} aria-hidden="true" />
                  </button>
                )}
              </div>
            </li>
          );
        })}
      </ul>
      <Recipes />
    </section>
  );
}

/** Saved prompts that can be run on demand or on a simple interval. */
function Recipes() {
  const [recipes, setRecipes] = useState<Recipe[]>([]);
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [prompt, setPrompt] = useState('');
  const [interval, setIntervalValue] = useState('');
  const [error, setError] = useState('');

  const refresh = useCallback(async () => {
    try {
      setRecipes((await api.recipesList()) ?? []);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const act = async (action: () => Promise<unknown>) => {
    setError('');
    try {
      await action();
      await refresh();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const add = async () => {
    const minutes = interval.trim() ? Number(interval) : null;
    await act(() => api.recipeSave({ name, prompt, intervalMinutes: minutes, enabled: true }));
    setName('');
    setPrompt('');
    setIntervalValue('');
  };

  const formatInterval = (recipe: Recipe) =>
    recipe.intervalMinutes ? `every ${recipe.intervalMinutes} min` : 'manual';

  return (
    <div className="task-inbox-recipes">
      <button type="button" className="task-inbox-toggle" onClick={() => setOpen(current => !current)} aria-expanded={open}>
        {open ? <ChevronDown size={12} aria-hidden="true" /> : <ChevronRight size={12} aria-hidden="true" />}
        <span>Recipes{recipes.length > 0 ? ` · ${recipes.length}` : ''}</span>
      </button>
      {open && (
        <>
          {error && <p role="alert" className="sidebar-error">{error}</p>}
          <ul className="task-inbox-list">
            {recipes.map(recipe => (
              <li key={recipe.id} className="task-inbox-item">
                <span className="task-inbox-text" title={recipe.prompt}>{recipe.name}</span>
                <div className="task-inbox-meta">
                  <span className="task-chip">{formatInterval(recipe)}</span>
                  {!recipe.enabled && <span className="task-inbox-reason">disabled</span>}
                  {recipe.nextRunAt && <span className="task-inbox-reason" title={new Date(recipe.nextRunAt * 1000).toLocaleString()}>next {new Date(recipe.nextRunAt * 1000).toLocaleTimeString()}</span>}
                </div>
                <div className="task-inbox-actions">
                  <button type="button" className="task-action" title="Queue a run now" aria-label={`Run recipe ${recipe.name}`} onClick={() => void act(() => api.recipeRunNow(recipe.id))}>
                    <Play size={11} aria-hidden="true" />
                  </button>
                  <button type="button" className="task-action task-action-danger" title="Delete recipe" aria-label={`Delete recipe ${recipe.name}`} onClick={() => void act(() => api.recipeDelete(recipe.id))}>
                    <Trash2 size={11} aria-hidden="true" />
                  </button>
                </div>
              </li>
            ))}
          </ul>
          <form className="recipe-form" onSubmit={event => { event.preventDefault(); void add(); }}>
            <input aria-label="Recipe name" placeholder="Recipe name" maxLength={120} value={name} onChange={event => setName(event.target.value)} />
            <input aria-label="Recipe prompt" placeholder="What it should do…" value={prompt} onChange={event => setPrompt(event.target.value)} />
            <input aria-label="Repeat every N minutes (blank for manual)" placeholder="Every N minutes (optional)" inputMode="numeric" value={interval} onChange={event => setIntervalValue(event.target.value)} />
            <button type="submit" className="secondary" disabled={!name.trim() || !prompt.trim()}>Save recipe</button>
          </form>
          <p className="settings-note">Scheduled recipes need enforced isolation (Settings → Execution); they disable themselves if it is turned off.</p>
        </>
      )}
    </div>
  );
}
