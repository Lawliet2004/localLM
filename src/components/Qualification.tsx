import { useCallback, useEffect, useState } from 'react';
import { CheckCircle2, CircleAlert, CirclePlay, FlaskConical, MinusCircle, XCircle } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { CaseResult, ModelSelection, Qualification, Readiness } from '../lib/types';

const STEP_LABELS: Record<string, string> = {
  'runtime-missing': 'Install a runtime',
  'model-missing': 'Choose a model',
  'not-loaded': 'Load the model',
  'loading': 'Loading',
  'load-failed': 'Load failed',
  'provider-unreachable': 'Provider unreachable',
  'not-qualified': 'Not qualified',
  'qualification-stale': 'Re-qualify',
  'ready': 'Ready',
};

/** One honest "what's missing" card — replaces scattered "load a model" hints. */
export function ReadinessCard({ onAction }: { onAction: (target: string) => void }) {
  const [readiness, setReadiness] = useState<Readiness | null>(null);
  useEffect(() => {
    if (!nativeAvailable) return;
    let alive = true;
    api.readiness().then(next => { if (alive) setReadiness(next); }).catch(() => {});
    return () => { alive = false; };
  }, []);
  if (!readiness || readiness.step === 'ready') return null;
  return (
    <div className="readiness-card" role="status">
      <div className="readiness-step">
        <span className="readiness-badge">{STEP_LABELS[readiness.step] ?? readiness.step}</span>
        <p>{readiness.message}</p>
      </div>
      {readiness.nextAction && (
        <button type="button" onClick={() => onAction(readiness.nextAction!.target)}>
          {readiness.nextAction.label}
        </button>
      )}
    </div>
  );
}

function caseIcon(status: string) {
  switch (status) {
    case 'passed': return <CheckCircle2 size={14} className="case-passed" aria-hidden="true" />;
    case 'failed': return <XCircle size={14} className="case-failed" aria-hidden="true" />;
    case 'unsupported': return <MinusCircle size={14} className="case-unsupported" aria-hidden="true" />;
    default: return <CircleAlert size={14} className="case-untested" aria-hidden="true" />;
  }
}

export function CaseRow({ result }: { result: CaseResult }) {
  return (
    <tr>
      <td>{result.name.replace(/_/g, ' ')}</td>
      <td>
        <span className={`case-chip ${result.status}`}>
          {caseIcon(result.status)} {result.status}
        </span>
      </td>
      <td>{result.latencyMs} ms</td>
      <td>{result.tokensPerSecond != null ? `${result.tokensPerSecond.toFixed(1)} tok/s` : '—'}</td>
      <td>{result.error ?? result.detail ?? ''}</td>
    </tr>
  );
}

/** Models page section: run the bounded suite, inspect each case. */
export function QualificationPanel({ selection, busy }: { selection: ModelSelection; busy: boolean }) {
  const [record, setRecord] = useState<Qualification | null>(null);
  const [running, setRunning] = useState(false);
  const [cases, setCases] = useState<CaseResult[]>([]);
  const [error, setError] = useState('');

  const refresh = useCallback(() => {
    if (!nativeAvailable) return;
    api.qualificationResults(selection).then(next => {
      setRecord(next);
      setCases(next?.cases ?? []);
    }).catch(() => {});
  }, [selection]);

  useEffect(() => { refresh(); }, [refresh]);

  async function run() {
    setError('');
    setRunning(true);
    setCases([]);
    try {
      await api.qualificationRun(selection, event => {
        if (event.kind === 'case') {
          if (event.result) {
            setCases(current => [...current.filter(c => c.name !== event.name), event.result!]);
          }
        } else {
          setRecord(event.record);
          setCases(event.record.cases);
        }
      });
      refresh();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setRunning(false);
    }
  }

  return (
    <section className="qualification-panel">
      <div className="settings-group-header">
        <h2>Qualification</h2>
        <p>Measured capabilities for the selected model — streaming, tool calls, structured output, cancellation. A load alone is not a pass.</p>
      </div>
      {error && <p className="error" role="alert">{error}</p>}
      {record?.stale && <p className="qualification-stale">This result predates a model or config change — it no longer describes the current model.</p>}
      <div className="qualification-actions">
        {running
          ? <button type="button" onClick={() => void api.qualificationCancel()}>Cancel run</button>
          : <button type="button" className="primary" disabled={busy} onClick={() => void run()}>
              <CirclePlay size={14} aria-hidden="true" /> Run qualification
            </button>}
        {record && (
          <span className={`qualification-overall ${record.status}`}>
            {record.status}{record.stale ? ' · stale' : ''}
          </span>
        )}
      </div>
      {cases.length > 0 && (
        <table className="qualification-table">
          <thead>
            <tr><th>Case</th><th>Result</th><th>Latency</th><th>Speed</th><th>Note</th></tr>
          </thead>
          <tbody>
            {cases.map(c => <CaseRow key={c.name} result={c} />)}
          </tbody>
        </table>
      )}
      {!record && !running && cases.length === 0 && (
        <p className="qualification-empty"><FlaskConical size={14} aria-hidden="true" /> No qualification run yet for this model.</p>
      )}
    </section>
  );
}
