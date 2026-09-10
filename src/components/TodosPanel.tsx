import { useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { Todo } from '../lib/types';

export function TodosPanel({ conversationId }: { conversationId: string | null }) {
  const [open, setOpen] = useState(false);
  const [todos, setTodos] = useState<Todo[]>([]);
  const [goal, setGoal] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  async function load() {
    if (!nativeAvailable || !conversationId || loading) return;
    setLoading(true); setError('');
    try {
      setTodos(await api.getTodos(conversationId));
      setGoal(await api.getGoal(conversationId));
    } catch (e) { setError(errorMessage(e)); } finally { setLoading(false); }
  }
  const done = todos.filter(todo => todo.status === 'completed').length;
  return (
    <details className="todos-panel" onToggle={event => {
      const isOpen = (event.target as HTMLDetailsElement).open;
      setOpen(isOpen);
      if (isOpen) void load();
    }}>
      <summary>Plan{todos.length ? ` · ${done}/${todos.length}` : ''}{goal ? ' · goal set' : ''}</summary>
      <div className="todos-body">
        <button type="button" className="secondary" disabled={loading || !conversationId} onClick={() => void load()}>{loading ? 'Loading…' : 'Refresh'}</button>
        {error && <p role="alert">{error}</p>}
        {!conversationId && <p>Start a conversation to track todos and goals. The model updates them with todo_write; this panel is read-only.</p>}
        {goal && <p><strong>Goal:</strong> {goal}</p>}
        {open && conversationId && !todos.length && !loading && !error && <p>No todos yet. Ask the model to plan the work.</p>}
        <ul>{todos.map((todo, index) => <li key={index}><strong>{todo.status}</strong> · {todo.text}</li>)}</ul>
      </div>
    </details>
  );
}
