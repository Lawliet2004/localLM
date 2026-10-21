import { useId } from 'react';
import * as C from './richCatalog';
import type { Props } from './componentTypes';

/** Bounded layered DAG layout. Cycles/back edges remain visible and are also
 * listed in the accessible alternative; no layout engine or generated code. */
export function Diagram({ props: p }: Props<typeof C.Diagram.props>) {
  const marker = useId().replace(/:/g, '');
  const levels = new Map(p.nodes.map(node => [node.id, 0]));
  const remaining = new Set(p.nodes.map(node => node.id));
  for (let pass = 0; pass < p.nodes.length; pass++) {
    const ready = [...remaining].filter(id => !p.edges.some(edge => edge.to === id && remaining.has(edge.from)));
    if (!ready.length) break;
    for (const id of ready) {
      levels.set(id, Math.max(0, ...p.edges.filter(e => e.to === id).map(e => (levels.get(e.from) ?? 0) + 1)));
      remaining.delete(id);
    }
  }
  const rows = new Map<number, string[]>();
  for (const node of p.nodes) { const level = levels.get(node.id)!; rows.set(level, [...(rows.get(level) ?? []), node.id]); }
  const width = Math.max(360, ...[...rows.values()].map(row => row.length * 180));
  const height = rows.size * 120 + 40;
  const location = (id: string) => {
    const level = levels.get(id) ?? 0, row = rows.get(level)!;
    return { x: ((row.indexOf(id) + 0.5) * width) / row.length, y: level * 120 + 40 };
  };
  return <figure className="genui-diagram"><figcaption>{p.title}</figcaption>
    <div className="genui-diagram-scroll"><svg viewBox={`0 0 ${width} ${height}`} style={{ minWidth: width }} role="img" aria-label={`${p.title}: ${p.nodes.length} nodes and ${p.edges.length} directed edges`}>
      <defs><marker id={marker} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" className="genui-diagram-arrow"/></marker></defs>
      {p.edges.map((edge, i) => { const from = location(edge.from), to = location(edge.to); return <g key={i}>
        <path d={from.y === to.y ? `M ${from.x} ${from.y+20} Q ${(from.x+to.x)/2} ${from.y+100} ${to.x} ${to.y+20}` : `M ${from.x} ${from.y+20} L ${to.x} ${to.y-20}`} className="genui-diagram-edge" markerEnd={`url(#${marker})`}/>
        {edge.label && <text x={(from.x+to.x)/2} y={(from.y+to.y)/2+35} textAnchor="middle" className="genui-diagram-label">{edge.label.slice(0,28)}<title>{edge.label}</title></text>}
      </g>; })}
      {p.nodes.map(node => { const pos = location(node.id); return <g key={node.id}><rect x={pos.x-78} y={pos.y-20} width="156" height="40" rx="8" className="genui-diagram-node"/><text x={pos.x} y={pos.y+5} textAnchor="middle" className="genui-diagram-label">{node.label.slice(0,20)}<title>{node.label}</title></text></g>; })}
    </svg></div>
    <details><summary>Nodes and connections</summary><ol>{p.nodes.map(node => <li key={node.id}><strong>{node.label}</strong>{node.detail && <p>{node.detail}</p>}<ul>{p.edges.filter(edge => edge.from === node.id).map((edge,i) => <li key={i}>To {p.nodes.find(n => n.id === edge.to)?.label}{edge.label ? ` · ${edge.label}` : ''}</li>)}</ul></li>)}</ol></details>
  </figure>;
}
