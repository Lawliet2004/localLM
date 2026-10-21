import { save } from '@tauri-apps/plugin-dialog';
import { useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
interface Record { task: string; mode: string; tools: boolean; cache: string; correct: boolean; complete: boolean; firstAnswerMs: number | null; totalMs: number; answer: string }
export function diagnosticRecords(value: unknown): Record[] {
  if (!value || typeof value !== 'object' || !('schemaVersion' in value) || value.schemaVersion !== 1 || !('records' in value) || !Array.isArray(value.records) || value.records.length > 1000) throw new Error('Choose a version 1 LocalLM diagnostic report with at most 1,000 records.');
  return value.records.map(record => {
    if (!record || typeof record !== 'object' || typeof record.task !== 'string' || typeof record.mode !== 'string' || typeof record.cache !== 'string' || typeof record.tools !== 'boolean' || typeof record.correct !== 'boolean' || typeof record.complete !== 'boolean' || typeof record.answer !== 'string' || typeof record.totalMs !== 'number' || !Number.isFinite(record.totalMs) || record.totalMs < 0 || record.firstAnswerMs !== null && (typeof record.firstAnswerMs !== 'number' || !Number.isFinite(record.firstAnswerMs) || record.firstAnswerMs < 0)) throw new Error('The diagnostic report contains invalid measurements.');
    return record as Record;
  });
}
export function percentile(values: number[], quantile: number) {
  const sorted = [...values].sort((a, b) => a - b);
  if (!sorted.length) return null;
  if (quantile === .5) return sorted.length % 2 ? sorted[(sorted.length - 1) / 2] : (sorted[sorted.length / 2 - 1] + sorted[sorted.length / 2]) / 2;
  return sorted[Math.max(0, Math.ceil(sorted.length * quantile) - 1)];
}
const milliseconds = (value: number | null) => value === null ? 'Unavailable' : `${(value / 1000).toFixed(2)}s`;
export function LatencyDiagnostic() {
  const [task, setTask] = useState('simple');
  const [mode, setMode] = useState<'auto' | 'quick' | 'think'>('auto');
  const [samples, setSamples] = useState(1);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [report, setReport] = useState<unknown>(null);
  const [saveNotice, setSaveNotice] = useState('');
  const records = report ? diagnosticRecords(report) : [];
  const groups = new Map<string, Record[]>();
  for (const record of records) if (record.cache !== 'prime') { const key = `${record.task} · ${record.mode} · ${record.tools ? 'tools on' : 'tools off'} · ${record.cache}`; groups.set(key, [...(groups.get(key) ?? []), record]); }
  return <section className="latency-diagnostic" aria-label="Response latency diagnostic"><h2>Response latency</h2><p>Run a bounded check on the loaded local model. Compare the same task with synthetic tool schemas on/off and requested cold/warm caches. This uses the current runtime and can replace its warm prompt cache. It runs no tools, generated code or live web search.</p>
    <div className="field-row"><label>Task<select value={task} disabled={busy} onChange={event => setTask(event.target.value)}><option value="simple">Simple answer</option><option value="coding">Code fixture</option><option value="research">Evidence fixture</option></select></label><label>Reasoning<select value={mode} disabled={busy} onChange={event => setMode(event.target.value as typeof mode)}><option value="auto">Auto</option><option value="quick">Quick</option><option value="think">Think</option></select></label><label>Samples per case<select value={samples} disabled={busy} onChange={event => setSamples(Number(event.target.value))}><option value={1}>1 — short check</option><option value={3}>3 — repeated check</option></select></label></div>
    <div className="field-row"><button type="button" className="secondary" disabled={!nativeAvailable || busy} onClick={async () => { setBusy(true); setError(''); try { const value = await api.runLatencyDiagnostic(task, mode, samples); diagnosticRecords(value); setReport(value); } catch (error) { setError(errorMessage(error)); } finally { setBusy(false); } }}>{busy ? 'Measuring response latency…' : 'Run local check'}</button>{busy && <button type="button" className="secondary" onClick={() => void api.cancelGeneration().catch(error => setError(errorMessage(error)))}>Cancel latency check</button>}<label>Load diagnostic report<input type="file" accept=".json,application/json" disabled={busy} onChange={async event => { const file = event.target.files?.[0]; if (!file) return; try { if (file.size > 4 * 1024 * 1024) throw Error('Report exceeds 4 MiB.'); const value: unknown = JSON.parse(await file.text()); diagnosticRecords(value); setReport(value); setError(''); } catch (error) { setError(errorMessage(error)); } }} /></label></div>
    {busy && <p role="status">Each request is limited to 160 tokens and 90 seconds. You can cancel at any time.</p>}
    {error && <p role="alert">{error}</p>}
    {saveNotice && <p role="status">{saveNotice}</p>}
    {records.length > 0 && <><p role="status">{records.filter(record => record.cache !== 'prime' && record.correct).length} of {records.filter(record => record.cache !== 'prime').length} measured fixtures passed correctness checks. {records.filter(record => !record.complete).length} incomplete streams. Quality failures are separate from latency.</p><div className="diagnostic-table" tabIndex={0} role="region" aria-label="Latency measurements"><table><thead><tr><th>Case</th><th>Correct</th><th>First answer median / p95</th><th>Total median / p95</th></tr></thead><tbody>{[...groups].map(([key, rows]) => { const complete = rows.filter(row => row.complete); const visible = complete.flatMap(row => row.firstAnswerMs === null ? [] : [row.firstAnswerMs]); const total = complete.map(row => row.totalMs); return <tr key={key}><th scope="row">{key}</th><td>{rows.filter(row => row.correct).length}/{rows.length}</td><td>{milliseconds(percentile(visible, .5))} / {milliseconds(percentile(visible, .95))}</td><td>{milliseconds(percentile(total, .5))} / {milliseconds(percentile(total, .95))}</td></tr>; })}</tbody></table></div><p>Cold means cache_prompt=false, not cold disk or a fresh process. First answer measures the HTTP client, before rendering. Small descriptive samples establish no causal speedup. Evidence fixtures use supplied fictional sources. Review answers alongside timings.</p><details><summary>Recorded answers and raw report</summary><pre tabIndex={0}>{JSON.stringify(report, null, 2)}</pre></details>{nativeAvailable ? <button className="secondary" onClick={async () => { try { const path = await save({ title: 'Save diagnostic report', defaultPath: 'locallm-latency.json' }); if (path) { await api.saveArtifactCopy(path, JSON.stringify(report), 'application/json'); setError(''); setSaveNotice(`Saved diagnostic report to ${path}`); } } catch (error) { setError(errorMessage(error)); } }}>Save diagnostic report</button> : <a className="secondary" download="locallm-latency.json" href={`data:application/json;charset=utf-8,${encodeURIComponent(JSON.stringify(report, null, 2))}`}>Save diagnostic report</a>}</>}
  </section>;
}
