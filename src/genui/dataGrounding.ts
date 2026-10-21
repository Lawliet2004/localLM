import { createParser } from '@openuidev/lang-core';
import { numericTables, scatterCoordinate, codeSamples } from './authoritativeData';
import { fullLibrary } from './specification';
const parser = createParser(fullLibrary.toJSONSchema(), 'Response');
const normalize = (s: string) => s.toLowerCase().replace(/(?<!\w)(?:\*\*|__)(?=\S)|(?<=\S)(?:\*\*|__)(?!\w)|`/g, '').replace(/\s+/g, ' ').trim();

/** Conservative provenance gate, not semantic proof. Each displayed row/metric
 * must be supported together in one authoritative clause/Markdown row. Code and
 * diffs must be literal substrings. Ambiguous prose falls back to the original.
 * Educational controls/simulations are explicitly labelled examples.
 */
export function validateChartGrounding(program: string, answer: string, evidence: Record<string, string> = {}): boolean {
  const boundCode = codeSamples(answer);
  const boundTables = numericTables(answer);
  const clauses = answer.split(/\n|;|(?<=[.!?])\s+/).map(normalize);
  const cells = (line: string) => line.trim().replace(/^\||\|$/g, '').split('|').map(normalize);
  const lines = answer.split('\n');
  const tables: {columns:string[];rows:string[][]}[] = [];
  for (let i=1;i<lines.length;i++) {
    if (!lines[i].includes('|') || !cells(lines[i]).every(cell => /^:?-{3,}:?$/.test(cell))) continue;
    const columns = cells(lines[i-1]), rows:string[][] = [];
    for (let j=i+1;j<lines.length && lines[j].includes('|');j++) { const row=cells(lines[j]); if (row.length !== columns.length) break; rows.push(row); }
    tables.push({columns,rows});
  }
  const supported = (cells: unknown[]) => clauses.some(clause => {
    let end = 0;
    for (const cell of cells) {
      const value = normalize(String(cell));
      if (cell === undefined || cell === null || !value) continue;
      const isNumber = typeof cell === 'number' || /^[-+]?\d+(?:\.\d+)?(?:e[-+]?\d+)?$/i.test(value);
      const start = clause.indexOf(value, end);
      if (isNumber && /\d/.test(clause.slice(end, start))) return false;
      if (start < 0) return false;
      // Numeric tokens must match exactly (10 is not supported by 100).
      if (isNumber && /[\d.]/.test(clause[start - 1] ?? ' ')) return false;
      if (isNumber && /^(?:\d|\.\d)/.test(clause.slice(start + value.length))) return false;
      end = start + value.length;
    }
    return true;
  });
  const genericTitles = new Set(['chart','data','values','supplied','comparison','overview','trend','results','distribution']);
  const titleSupported = (title: string) =>
    (title.toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? []).every(word => genericTitles.has(word) || (/^[a-z0-9]+$/.test(word) ? new RegExp(`\\b${word}\\b`, 'i').test(answer) : normalize(answer).includes(word)) || (word === 'sales' && /\bsold\b/i.test(answer))) &&
    !['$', '€', '£', '%'].some(unit => title.includes(unit) && !answer.includes(unit));
  const genericColumns = new Set(['month','year','date','label','value','name','item','category','description','details','question','answer','type']);
  const columnSupported = (column: string) => genericColumns.has(normalize(column)) || normalize(answer).includes(normalize(column));
  try {
    const pending: unknown[] = [parser.parse(program).root]; let count = 0;
    while (pending.length) {
      if (++count > 8192) return false;
      const item = pending.pop();
      if (!item || typeof item !== 'object') continue;
      if (Array.isArray(item)) { pending.push(...item); continue; }
      const node = item as { type?: string; typeName?: string; props?: Record<string, unknown> };
      if (node.type === 'element') {
        const p = node.props!;
        switch (node.typeName) {
          case 'EvidenceView': if (!(p.citationIds as string[]).every(id => Object.prototype.hasOwnProperty.call(evidence,id) && !!evidence[id])) return false; break;
          case 'SeriesChart': if (!(p.series as {name:string;points:{label:string;value:number}[]}[]).every(series => series.points.every(point => supported([series.name,point.label,point.value])))) return false; break;
          case 'SourceCode': if (!boundCode[Number(p.fenceIndex)] || (p.path && boundCode[Number(p.fenceIndex)].path!==p.path)) return false; break;
          case 'SourceChart': {
            const table=boundTables[Number(p.tableIndex)];
            if(!table || (['pie','donut'].includes(String(p.kind)) && (!table.points.every(point=>point.value>=0)||!table.points.some(point=>point.value>0))) || (p.kind==='scatter'&&table.points.some(point=>scatterCoordinate(point.label)===undefined)))return false;
            break;
          }
          case 'Chart':
            if ([p.xLabel,p.yLabel].some(label => label && !normalize(answer).includes(normalize(String(label))))) return false;
            if (!titleSupported(String(p.title)) || !(p.points as { label: string; value: number; x?: number }[]).every(point => supported(point.x === undefined ? [point.label, point.value] : [point.label, point.x, point.value]))) return false;
            break;
          case 'Table': {
            const columns = p.columns as string[], rows = p.rows as unknown[][];
            if (!columns.every(columnSupported) || !rows.every(row => {
              const typed = tables.some(table => table.columns.length===columns.length && table.columns.every((column,i)=>column===normalize(columns[i])) && table.rows.some(source => source.length===row.length && source.every((cell,i)=>cell===normalize(String(row[i])))));
              if (typed) return true;
              if (!supported(row)) return false;
              return row.every((cell,i) => i===0 || genericColumns.has(normalize(columns[i])) || supported([row[0],columns[i],cell]));
            })) return false;
            break;
          }
          case 'Stat': if (!supported([p.label, p.value, p.detail])) return false; break;
          case 'KeyValue': if (!(p.entries as { label: string; value: string }[]).every(row => supported([row.label, row.value]))) return false; break;
          case 'Progress': if (!supported([p.label, p.percent, '%', p.detail])) return false; break;
          case 'FilePreview': if (!answer.includes(String(p.path)) || !answer.includes(String(p.code)) || (boundCode.length>0 && !boundCode.some(sample=>sample.path===p.path&&(sample.code===p.code||sample.code===String(p.code)+'\n')))) return false; break;
          case 'FileTree': if (!(p.paths as string[]).every(path => answer.includes(path))) return false; break;
          case 'CodeBlock': if (!answer.includes(String(p.code))) return false; break;
          case 'DiffViewer': if (!answer.includes(String(p.diff))) return false; break;
          case 'SourceCard': {
            const id = String(p.citationId);
            if (!Object.prototype.hasOwnProperty.call(evidence,id) || !evidence[id] || !clauses.some(clause => {
              const marker=`[${id.toLowerCase()}]`,at=clause.indexOf(marker);if(at<0)return false;
              const prefixStyle=clause.startsWith('[');
              const segment=prefixStyle?clause.slice(at+marker.length).split(/\[[a-z0-9_-]+\]/i)[0]:clause.slice(0,at).split(/\[[a-z0-9_-]+\]/i).pop()??'';
              return segment.includes(normalize(String(p.title)))&&(!p.summary||segment.includes(normalize(String(p.summary))));
            })) return false;
            break;
          }
          case 'Timeline': if (!(p.events as { date: string; title: string; detail?: string }[]).every(e => supported([e.date, e.title, e.detail]))) return false; break;
        }
      }
      pending.push(...Object.values(item));
    }
    return true;
  } catch { return false; }
}
