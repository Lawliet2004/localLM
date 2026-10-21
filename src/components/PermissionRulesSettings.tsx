import { useEffect, useState } from 'react';
import { Plus, Trash2 } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import type { PermissionRuleView } from '../lib/types';

const ACTIONS: PermissionRuleView['action'][] = ['command', 'path', 'tool', 'host'];
const EFFECTS: PermissionRuleView['effect'][] = ['allow', 'ask', 'deny'];

const HINTS: Record<PermissionRuleView['action'], string> = {
  command: 'A shell command prefix. "cargo test*" allows cargo test and cargo test --all. A trailing " *" also matches the bare command.',
  path: 'A workspace path. "**" crosses directories, "*" does not. "src/**" matches every file under src.',
  tool: 'A tool name, optionally with a trailing "*" to cover a family.',
  host: 'A web host. "*.example.com" matches api.example.com but not example.com.',
};

/**
 * Rule-based permissions.
 *
 * The access mode decides the calls no rule matched. A rule is the user's
 * explicit instruction and is evaluated first: a deny refuses the call even
 * under full access, an allow permits it without a prompt, and an ask forces
 * a prompt even under full access. Rules are never written by the model.
 */
export function PermissionRulesSettings() {
  const [rules, setRules] = useState<PermissionRuleView[]>([]);
  const [status, setStatus] = useState('');
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let live = true;
    void api.listPermissionRules()
      .then(value => { if (live) { setRules(value); setLoaded(true); } })
      .catch(error => { if (live) { setStatus(errorMessage(error)); setLoaded(true); } });
    return () => { live = false; };
  }, []);

  const save = async (next: PermissionRuleView[]) => {
    setBusy(true);
    setStatus('');
    try {
      await api.savePermissionRules(next);
      setRules(next);
      setStatus('Rules saved.');
    } catch (error) {
      setStatus(errorMessage(error));
    } finally {
      setBusy(false);
    }
  };

  const update = (index: number, patch: Partial<PermissionRuleView>) => {
    setRules(current => current.map((rule, position) => position === index ? { ...rule, ...patch } : rule));
  };

  if (!loaded) return <section className="settings-section"><p className="panel-empty">Loading rules…</p></section>;

  return <section className="settings-section permission-rules">
    <h2>Permission rules</h2>
    <p className="settings-hint">
      Rules are checked before the access mode. The last matching rule wins. A rule can only narrow what the
      access mode would allow on its own when it says <strong>allow</strong>; the model can never write a rule.
    </p>
    {rules.length === 0
      ? <p className="panel-empty">No rules. Every call follows the access mode.</p>
      : <ul className="permission-rule-list">
        {rules.map((rule, index) => <li key={index}>
          <select aria-label={`Rule ${index + 1} action`} value={rule.action} onChange={event => update(index, { action: event.target.value as PermissionRuleView['action'] })}>
            {ACTIONS.map(action => <option key={action} value={action}>{action}</option>)}
          </select>
          <input aria-label={`Rule ${index + 1} resource`} value={rule.resource} placeholder="pattern" title={HINTS[rule.action]} onChange={event => update(index, { resource: event.target.value })} />
          <select aria-label={`Rule ${index + 1} effect`} value={rule.effect} onChange={event => update(index, { effect: event.target.value as PermissionRuleView['effect'] })}>
            {EFFECTS.map(effect => <option key={effect} value={effect}>{effect}</option>)}
          </select>
          <button className="icon-button" aria-label={`Remove rule ${index + 1}`} onClick={() => void save(rules.filter((_, position) => position !== index))}><Trash2 size={15} /></button>
        </li>)}
      </ul>}
    <p className="settings-hint">{HINTS[rules[0]?.action ?? 'command']}</p>
    <div className="settings-actions">
      <button className="secondary" disabled={busy} onClick={() => setRules(current => [...current, { action: 'command', resource: '', effect: 'allow' }])}><Plus size={15} /> Add rule</button>
      <button className="primary" disabled={busy || !rules.length} onClick={() => void save(rules)}>Save rules</button>
    </div>
    {status && <p className="settings-status" role="status">{status}</p>}
  </section>;
}
