import { useEffect, useState } from 'react';
import { Check, ChevronDown, ChevronRight, Circle, LoaderCircle } from 'lucide-react';
import { api, nativeAvailable } from '../lib/api';
import type { SessionEvent, TodoItem } from '../lib/types';

export function isPlanStateTool(name: string) {
  return name === 'todo_write' || name === 'todo_add' || name === 'todo_update' || name === 'goal_set' || name === 'goal_clear';
}

export function parsePlanSteps(planText: string): TodoItem[] {
  const todos: TodoItem[] = [];
  for (const raw of planText.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    const step = stepText(line);
    if (!step) continue;
    const text = step.replace(/[*_`]/g, '').replace(/\s+/g, ' ').trim();
    if (text.length < 3 || text.length > 500) continue;
    todos.push({
      text,
      status: todos.length === 0 ? 'in_progress' : 'pending',
      updatedAt: 0,
    });
    if (todos.length >= 50) break;
  }
  return todos;
}

function stepText(line: string): string | null {
  const numbered = line.match(/^\d+[.):]\s+(.*)$/);
  if (numbered) return numbered[1];
  const bullet = line.match(/^[-*+–—]\s+(.*)$/);
  return bullet ? bullet[1] : null;
}

export function todosFromPlanEvents(events: SessionEvent[]): TodoItem[] {
  for (let i = events.length - 1; i >= 0; i--) {
    if (events[i].eventType !== 'plan_created') continue;
    const content = (events[i].payload as { content?: unknown } | null)?.content;
    if (typeof content === 'string' && content.trim()) return parsePlanSteps(content);
  }
  return [];
}

export function useConversationTodos(conversationId: string, generating: boolean, fallback: TodoItem[] = []) {
  const [todos, setTodos] = useState<TodoItem[]>([]);
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    if (!nativeAvailable || conversationId === 'new' || !api.getTodos) {
      setTodos([]);
      return;
    }
    async function load() {
      try {
        const items = await api.getTodos(conversationId);
        if (!disposed) setTodos(items);
      } catch {
        if (!disposed) setTodos([]);
      }
      if (!disposed && generating) timer = setTimeout(() => void load(), 500);
    }
    void load();
    return () => {
      disposed = true;
      if (timer) clearTimeout(timer);
    };
  }, [conversationId, generating]);
  return todos.length ? todos : fallback;
}

function statusLabel(status: string) {
  if (status === 'completed') return 'Completed';
  if (status === 'in_progress') return 'In progress';
  return 'Pending';
}

export function PlanChecklist({
  todos,
  generating = false,
  drafting = false,
  defaultOpen,
  conversationId,
}: {
  todos: TodoItem[];
  generating?: boolean;
  drafting?: boolean;
  defaultOpen?: boolean;
  conversationId?: string;
}) {
  const done = todos.filter(todo => todo.status === 'completed').length;
  const current = todos.find(todo => todo.status === 'in_progress') ?? todos.find(todo => todo.status === 'pending');
  const allDone = todos.length > 0 && done === todos.length;
  const shouldOpen = defaultOpen ?? (generating || drafting || Boolean(current && !allDone));
  const [open, setOpen] = useState(shouldOpen);
  // A finished/abandoned plan can outlive its run; "Clear" drops the durable
  // rows and hides the section until a new plan or conversation appears.
  const [cleared, setCleared] = useState(false);

  useEffect(() => {
    if (shouldOpen) setOpen(true);
  }, [shouldOpen]);

  useEffect(() => {
    setCleared(false);
  }, [conversationId]);

  useEffect(() => {
    if (generating) setCleared(false);
  }, [generating]);

  if (cleared || (!todos.length && !drafting)) return null;

  const canClear = Boolean(
    conversationId && conversationId !== 'new' && nativeAvailable && !generating && !drafting && todos.length,
  );
  const clearPlan = () => {
    setCleared(true);
    if (conversationId) void api.clearTodos(conversationId).catch(() => setCleared(false));
  };

  const percent = todos.length ? Math.round((done / todos.length) * 100) : 0;
  const countLabel = drafting && !todos.length
    ? 'Drafting plan'
    : allDone
    ? `${done} of ${todos.length} done`
    : `${done} of ${todos.length}`;

  return (
    <section className={`plan-checklist ${open ? 'open' : 'collapsed'} ${allDone ? 'complete' : ''} ${generating || drafting ? 'live' : ''}`} aria-label="Plan checklist">
      <button
        type="button"
        className="plan-checklist-toggle"
        onClick={() => setOpen(value => !value)}
        aria-expanded={open}
      >
        <span className="plan-checklist-chevron" aria-hidden="true">
          {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </span>
        <span className="plan-checklist-title">Plan</span>
        {' '}
        <span className="plan-checklist-count">{countLabel}</span>
        {todos.length > 0 && (
          <span className="plan-checklist-meter" aria-hidden="true">
            <span className="plan-checklist-meter-fill" style={{ width: `${percent}%` }} />
          </span>
        )}
        {!open && current && !allDone && (
          <span className="plan-checklist-current-preview">{current.text}</span>
        )}
      </button>
      {open && (
        <ol className="plan-checklist-items">
          {drafting && !todos.length && (
            <li className="plan-checklist-item in_progress">
              <span className="plan-checklist-mark" aria-hidden="true">
                <LoaderCircle size={14} className="plan-checklist-spin" />
              </span>
              <span className="plan-checklist-text">Writing the checklist…</span>
            </li>
          )}
          {todos.map((todo, index) => {
            const status = todo.status === 'completed' || todo.status === 'in_progress' ? todo.status : 'pending';
            return (
              <li key={`${index}-${todo.text}`} className={`plan-checklist-item ${status}`}>
                <span className="plan-checklist-mark" aria-hidden="true">
                  {status === 'completed' ? (
                    <span className="plan-checklist-check"><Check size={11} strokeWidth={3} /></span>
                  ) : status === 'in_progress' ? (
                    generating ? <LoaderCircle size={14} className="plan-checklist-spin" /> : <Circle size={14} className="plan-checklist-current-dot" />
                  ) : (
                    <Circle size={14} />
                  )}
                </span>
                <span className="plan-checklist-text">
                  <span className="sr-only">{statusLabel(status)}. </span>
                  {todo.text}
                </span>
                {status === 'in_progress' && <span className="plan-checklist-now">Now</span>}
              </li>
            );
          })}
        </ol>
      )}
      {open && canClear && (
        <div className="plan-checklist-footer">
          <button type="button" className="plan-checklist-clear" onClick={clearPlan}>
            Clear plan
          </button>
        </div>
      )}
    </section>
  );
}
