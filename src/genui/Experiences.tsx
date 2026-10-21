import { useContext } from 'react';
import { useIsStreaming, useStateField } from '@openuidev/react-lang';
import * as C from './richCatalog';
import type { Props } from './componentTypes';
import { ResearchContext, AuthoritativeDataContext, AuthoritativeCodeContext, useEvidence } from './grounding';
import { safeSourceUrl } from '../lib/sources';
import { scatterCoordinate } from "./authoritativeData";
import { FilePreview } from "./Coding";
import { Chart } from './Chart';
export function FunctionPlot({props:p}: Props<typeof C.FunctionPlot.props>) {
  const field = useStateField<number>(p.parameterId);
  const coefficient = Number(field.value);
  if (field.value === undefined) return <p role="status">Building linked example…</p>;
  const points = Array.from({length:41}, (_, i) => {
    const x = p.minX + (p.maxX-p.minX)*i/40;
    return {label:x.toFixed(2), x, value:p.family === 'linear' ? coefficient*x : p.family === 'quadratic' ? coefficient*x*x : Math.exp(coefficient*x)};
  });
  if (!Number.isFinite(coefficient) || points.some(point=>!Number.isFinite(point.value)||Math.abs(point.value)>1e12)) return <p role="status">Parameter produces values outside the supported finite range.</p>;
  return <section><p className="genui-muted">Educational function: y = {p.family === 'linear' ? 'a × x' : p.family === 'quadratic' ? 'a × x²' : 'exp(a × x)'}. a = {coefficient}. Adjust the linked slider.</p><Chart props={{title:p.title,kind:'line',points}}/></section>;
}
export function EvidenceView({props:p}: Props<typeof C.EvidenceView.props>) {
  const links = useEvidence(), host = useContext(ResearchContext);
  return <section aria-label="Collected evidence and claim audit"><h3>Collected evidence</h3><ul>{p.citationIds.map(id=>{
    const url = safeSourceUrl(links[id]);
    const source = host.sources.find(source=>source.ids.includes(id) && source.url === links[id]);
    return <li key={id}>{url ? <><a href={url.href} target="_blank" rel="noopener noreferrer">[{id}] {source?.title ?? url.hostname}</a>{source?.date && <time>{source.date}</time>}{source?.snippet && <p>{source.snippet}</p>}</> : <span>Unavailable evidence [{id}]</span>}</li>;
  })}</ul><h4>Claim audit</h4>{host.claims.length ? <table className="genui-table"><thead><tr><th>Claim</th><th>Lexical result</th><th>Evidence IDs</th></tr></thead><tbody>{host.claims.map((claim,i)=><tr key={i}><td>{claim.claim}</td><td>{claim.verdict}</td><td>{claim.citations.filter(id=>!!links[id]).join(', ') || 'None'}</td></tr>)}</tbody></table> : <p>No host claim audit is available for this answer.</p>}<p className="genui-muted">Collected metadata and deterministic lexical matching; not independent verification.</p></section>;
}
export function SeriesChart({props:p}: Props<typeof C.SeriesChart.props>) {
  const hidden = useStateField<Record<string, boolean>>(`series-${p.title}`, {}), streaming = useIsStreaming();
  const values = p.series.flatMap(series => series.points.map(point => point.value));
  const domain: [number, number] = [Math.min(...values), Math.max(...values)];
  return <section><h3>{p.title}</h3><fieldset disabled={streaming}><legend>Visible series</legend>{p.series.map(series=><label className="genui-choice" key={series.name}><input type="checkbox" checked={!hidden.value?.[series.name]} onChange={e=>hidden.setValue({...hidden.value,[series.name]:!e.target.checked})}/>{series.name}</label>)}</fieldset><div className="genui-columns">{p.series.filter(series=>!hidden.value?.[series.name]).map(series=><Chart key={series.name} domain={domain} props={{title:`${p.title} · ${series.name}`,kind:'line',points:series.points}}/>)}</div></section>;
}

export function SourceChart({props:p}: Props<typeof C.SourceChart.props>) {
  const tables=useContext(AuthoritativeDataContext),table=tables[p.tableIndex];
  if(!table || (['pie','donut'].includes(p.kind)&&(!table.points.every(point=>point.value>=0)||!table.points.some(point=>point.value>0))) || (p.kind==='scatter'&&table.points.some(point=>scatterCoordinate(point.label)===undefined)))return <p role="status">No compatible authoritative chart table is available.</p>;
  const points=table.points.map(point=>({...point,...(p.kind==='scatter'?{x:scatterCoordinate(point.label)}:{})}));
  return <Chart stateId={`host-chart-${p.id}`} props={{title:table.columns[1],kind:p.kind,points,xLabel:table.columns[0],yLabel:table.columns[1]}}/>;
}

export function SourceCode({props:p}: Props<typeof C.SourceCode.props>) {
 const code=useContext(AuthoritativeCodeContext)[p.fenceIndex];
 return code?<FilePreview stateId={`source-code-${p.fenceIndex}`} props={{path:code.path??`Original code block ${p.fenceIndex+1}`,code:code.code,language:code.language}}/>:<p role="status">No original code block is available.</p>;
}
