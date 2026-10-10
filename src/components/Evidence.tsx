import { useCallback, useEffect, useState } from 'react';
import { CheckCircle2, ChevronRight, CircleAlert, CircleHelp, FileText, FlaskConical, Play, Plus, Trash2, X } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { TaskCriterion, VerificationRecord, VerificationSummary } from '../lib/types';

const STATE_LABEL: Record<string, { label: string; tone: string }> = {
  'work-completed': { label: 'Work completed · not verified', tone: 'muted' },
  'checks-passed': { label: 'Checks passed', tone: 'ok' },
  'checks-failed': { label: 'Checks failed', tone: 'bad' },
  'partially-verified': { label: 'Partially verified', tone: 'warn' },
  'needs-review': { label: 'Needs review', tone: 'warn' },
};

const ENV_LABEL: Record<string, string> = {
  'native-local': 'local',
  'task-worktree': 'task worktree',
  'snapshot': 'snapshot copy',
  'mocked': 'mocked',
};

const RESULT_LABEL: Record<string, string> = {
  passed: 'passed',
  failed: 'failed',
  unavailable: 'unavailable',
  inconclusive: 'inconclusive',
};

function timeLabel(at: number): string {
  if (!at) return '';
  return new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function RecordRow({ record, criteria }: { record: VerificationRecord; criteria: TaskCriterion[] }) {
  const [log, setLog] = useState<{ text: string; truncated: boolean; note?: string } | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const criterion = record.criterionId ? criteria.find(item => item.id === record.criterionId) : null;
  const showLog = async () => {
    if (log) {
      setLog(null);
      return;
    }
    setLoading(true);
    setError('');
    try {
      setLog(await api.verificationRecordOutput(record.id));
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setLoading(false);
    }
  };
  return (
    <li className={`evidence-record evidence-${record.result}`}>
      <div className="evidence-record-head">
        <span className={`evidence-result evidence-result-${record.result}`}>{RESULT_LABEL[record.result] ?? record.result}</span>
        <span className="evidence-method">{record.method}</span>
        <span className="evidence-env">{ENV_LABEL[record.environment] ?? record.environment}</span>
        {record.stale && <span className="evidence-stale" title="The workspace changed after this check ran — rerun it.">stale</span>}
        {record.exitCode !== null && record.exitCode !== undefined && <code className="evidence-exit">exit {record.exitCode}</code>}
        <span className="evidence-time">{timeLabel(record.endedAt || record.createdAt)}</span>
        <button type="button" className="evidence-log-btn" onClick={() => void showLog()} disabled={loading} aria-expanded={log !== null}>
          <FileText size={12} aria-hidden="true" /> {log ? 'Hide log' : 'Show full log'}
        </button>
      </div>
      {record.command && <code className="evidence-command">{record.command}</code>}
      {criterion && <p className="evidence-record-criterion">For: {criterion.text}</p>}
      {record.limitations && <p className="evidence-limitation">{record.limitations}</p>}
      {record.outputExcerpt && !log && <pre className="evidence-excerpt" aria-label="Output excerpt">{record.outputExcerpt}</pre>}
      {log && (
        <div className="evidence-log">
          {log.note && <p className="evidence-limitation">{log.note}</p>}
          <pre aria-label="Full check log">{log.text}</pre>
        </div>
      )}
      {error && <p className="error-banner" role="alert">{error}</p>}
    </li>
  );
}

function CriterionRow({ criterion, covered, onChanged }: { criterion: TaskCriterion; covered: boolean; onChanged: () => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(criterion.text);
  const [error, setError] = useState('');
  const save = async () => {
    setError('');
    try {
      await api.criteriaUpsert(criterion.conversationId, criterion.id, draft);
      setEditing(false);
      onChanged();
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };
  const remove = async () => {
    setError('');
    try {
      await api.criteriaDelete(criterion.conversationId, criterion.id);
      onChanged();
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };
  return (
    <li className="evidence-criterion">
      {editing
        ? <>
            <input value={draft} onChange={event => setDraft(event.target.value)} maxLength={500}
              aria-label="Edit criterion" onKeyDown={event => { if (event.key === 'Enter') void save(); }} />
            <button type="button" onClick={() => void save()}><CheckCircle2 size={13} aria-hidden="true" /> Save</button>
            <button type="button" className="icon-button" aria-label="Cancel edit" onClick={() => { setEditing(false); setDraft(criterion.text); }}><X size={13} /></button>
          </>
        : <>
            <span className={`evidence-covered ${covered ? 'is-covered' : ''}`} aria-label={covered ? 'Verified by a passing check' : 'No passing check yet'}>
              {covered ? <CheckCircle2 size={13} aria-hidden="true" /> : <CircleHelp size={13} aria-hidden="true" />}
            </span>
            <span className="evidence-criterion-text">{criterion.text}</span>
            {criterion.origin === 'agent' && <span className="evidence-origin" title="The agent proposed this criterion — review whether it actually proves the work.">agent</span>}
            <button type="button" className="evidence-criterion-edit" onClick={() => setEditing(true)}>Edit</button>
            <button type="button" className="icon-button" aria-label={`Delete criterion: ${criterion.text}`} onClick={() => void remove()}><Trash2 size={13} /></button>
          </>}
      {error && <p className="error-banner" role="alert">{error}</p>}
    </li>
  );
}

/// Evidence block under the latest assistant answer: what was promised vs.
/// what was actually checked. Collapsed by default; the badge is honest —
/// "not verified" rather than green-by-default.
export function Evidence({ conversation, tick }: { conversation: string; tick: number }) {
  const [summary, setSummary] = useState<VerificationSummary | null>(null);
  const [open, setOpen] = useState(false);
  const [newCriterion, setNewCriterion] = useState('');
  const [check, setCheck] = useState('');
  const [checkCriterion, setCheckCriterion] = useState('');
  const [running, setRunning] = useState(false);
  const [error, setError] = useState('');

  const refresh = useCallback(() => {
    if (!nativeAvailable || conversation === 'new') return;
    api.verificationSummary(conversation).then(setSummary).catch(() => setSummary(null));
  }, [conversation]);

  useEffect(refresh, [refresh, tick]);

  if (!nativeAvailable || conversation === 'new' || !summary) return null;
  const { state } = summary;
  const badge = STATE_LABEL[state.state] ?? { label: state.state, tone: 'muted' };
  const coveredIds = new Set(
    summary.records.filter(record => record.result === 'passed' && !record.stale && record.criterionId).map(record => record.criterionId),
  );
  const addCriterion = async () => {
    setError('');
    try {
      await api.criteriaUpsert(conversation, null, newCriterion);
      setNewCriterion('');
      refresh();
    } catch (caught) {
      setError(errorMessage(caught));
    }
  };
  const runCheck = async () => {
    setRunning(true);
    setError('');
    try {
      await api.verificationRunUser(conversation, check, checkCriterion || undefined);
      setCheck('');
      refresh();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setRunning(false);
    }
  };
  return (
    <div className="evidence">
      <button type="button" className="evidence-toggle" aria-expanded={open} onClick={() => setOpen(value => !value)}>
        <ChevronRight size={13} className={`evidence-chevron ${open ? 'is-open' : ''}`} aria-hidden="true" />
        <FlaskConical size={13} className="evidence-icon" aria-hidden="true" />
        <span className={`evidence-badge evidence-badge-${badge.tone}`}>{badge.label}</span>
        <span className="evidence-state-summary">{state.summary}</span>
      </button>
      {open && (
        <div className="evidence-body">
          <p className="evidence-hint">Completion is judged by these records, not by the model's wording.</p>
          <div className="evidence-section">
            <p className="evidence-section-label">Criteria</p>
            {summary.criteria.length === 0 && <p className="evidence-empty">No criteria yet — add one or let the agent propose some.</p>}
            <ul className="evidence-criteria-list">
              {summary.criteria.map(criterion => (
                <CriterionRow key={criterion.id} criterion={criterion} covered={coveredIds.has(criterion.id)} onChanged={refresh} />
              ))}
            </ul>
            <div className="evidence-add">
              <input value={newCriterion} onChange={event => setNewCriterion(event.target.value)} maxLength={500}
                placeholder="Add a criterion, e.g. “tests pass”" aria-label="New criterion"
                onKeyDown={event => { if (event.key === 'Enter') void addCriterion(); }} />
              <button type="button" onClick={() => void addCriterion()} disabled={!newCriterion.trim()}>
                <Plus size={13} aria-hidden="true" /> Add
              </button>
            </div>
          </div>
          <div className="evidence-section">
            <p className="evidence-section-label">Checks</p>
            {summary.records.length === 0 && <p className="evidence-empty">No checks have run.</p>}
            <ul className="evidence-records-list">
              {summary.records.map(record => <RecordRow key={record.id} record={record} criteria={summary.criteria} />)}
            </ul>
            <div className="evidence-run">
              <input value={check} onChange={event => setCheck(event.target.value)} maxLength={4096}
                placeholder="Run a check, e.g. npm test" aria-label="Check command" disabled={running}
                onKeyDown={event => { if (event.key === 'Enter' && check.trim()) void runCheck(); }} />
              {summary.criteria.length > 0 && (
                <select value={checkCriterion} onChange={event => setCheckCriterion(event.target.value)} aria-label="Link check to criterion">
                  <option value="">No criterion</option>
                  {summary.criteria.map(criterion => <option key={criterion.id} value={criterion.id}>{criterion.text}</option>)}
                </select>
              )}
              <button type="button" onClick={() => void runCheck()} disabled={running || !check.trim()}>
                <Play size={13} aria-hidden="true" /> {running ? 'Running…' : 'Run check'}
              </button>
            </div>
          </div>
          {error && <p className="error-banner" role="alert">{error}</p>}
          <p className="evidence-note"><CircleAlert size={12} aria-hidden="true" /> Passing checks are evidence, not proof — review the criteria and the log yourself.</p>
        </div>
      )}
    </div>
  );
}
