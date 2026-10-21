/** Version-1 extraction for host-bound charts. No inference or unit conversion.
 * Only complete two-column Markdown tables with literal finite numeric cells
 * qualify. IDs index these tables in source order; answer edits invalidate UI. */
export interface NumericTable { columns: [string,string]; points: {label:string;value:number}[] }
const number = /^[-+]?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?(?:e[-+]?\d+)?$/i;
const clean = (cell:string) => cell.trim().replace(/^`(.*)`$/, '$1').replace(/^\*\*(.*)\*\*$/, '$1');
const cells = (line:string) => line.trim().replace(/^\||\|$/g,'').split('|').map(clean);
export function numericTables(answer:string): NumericTable[] {
  const lines=answer.split(/\r?\n/,2049), tables:NumericTable[]=[];
  if(lines.length>2048)return tables;
  for(let i=1;i<lines.length && tables.length<8;i++) {
    if(!lines[i].includes('|'))continue;
    const separator=cells(lines[i]),columns=cells(lines[i-1]);
    if(separator.length!==2||columns.length!==2||!separator.every(cell=>/^:?-{3,}:?$/.test(cell))||columns.some(cell=>!cell||cell.length>240))continue;
    const points:NumericTable['points']=[];let valid=true;
    for(let j=i+1;j<lines.length&&lines[j].includes('|');j++) {
      const row=cells(lines[j]);
      if(row.length!==2||!row[0]||row[0].length>240||!number.test(row[1])||points.length>=100){valid=false;break;}
      const value=Number(row[1].replace(/,/g,''));
      if(!Number.isFinite(value)||Math.abs(value)>1e12){valid=false;break;}
      points.push({label:row[0],value});
    }
    if(valid&&points.length)tables.push({columns:columns as [string,string],points});
  }
  // Colon bullets are typed associations too. Keep unit groups separate and
  // never infer values from prose, ASCII bar lengths or arithmetic summaries.
  let group:NumericTable['points']=[],unit='',overflow=false;
  const flush=()=>{if(group.length&&!overflow&&tables.length<8)tables.push({columns:['Label',unit||'Value'],points:group});group=[];unit='';overflow=false;};
  for(const line of lines) {
    const item=/^\s*[-*+]\s+([^:]{1,240}):\s*([-+]?(?:\d{1,3}(?:,\d{3})+|\d+)(?:\.\d+)?(?:e[-+]?\d+)?)\s*([^\d():]{0,64})$/iu.exec(line.replace(/\*\*/g,''));
    if(!item){flush();continue;}
    const label=clean(item[1]),nextUnit=item[3].trim(),value=Number(item[2].replace(/,/g,''));
    if(!label||label.length>240||!Number.isFinite(value)||Math.abs(value)>1e12){flush();continue;}
    if(group.length&&nextUnit!==unit)flush();unit=nextUnit;
    if(group.length>=100)overflow=true;else group.push({label,value});
  }
  flush();
  return tables;
}
export function scatterCoordinate(label:string):number|undefined {
  if(!number.test(label))return;
  const value=Number(label.replace(/,/g,''));
  return Number.isFinite(value)&&Math.abs(value)<=1e12?value:undefined;
}

export interface CodeSample { code:string; language:string; path?:string }
/** Closed fences only, retaining literal bytes including the final newline. */
export function codeSamples(answer:string):CodeSample[] {
 const samples:CodeSample[]=[];let open:{marker:string;start:number;language:string;path?:string}|undefined,lines=0;
 for(const match of answer.matchAll(/^ {0,3}(`{3,}|~{3,})([^\r\n]*)\r?$/gm)) {
  if(++lines>2048)return [];
  if(!open) {
   const language=match[2].trim().split(/\s+/)[0].toLowerCase();
   const previous=answer.slice(Math.max(0,match.index!-1024),match.index).trimEnd().split(/\r?\n/).pop()?.trim().replace(/^#{1,6}\s+/,'').replace(/[:\s]+$/,'').replace(/^\*\*(.*)\*\*$/,'$1').replace(/^`(.*)`$/,'$1')??'';
   const named=previous; // Only explicit filename headings establish file ownership.
   const path=named.length<=240&&/^[a-z0-9_.\/-]+\.[a-z0-9]{1,16}$/i.test(named)?named:undefined;
   open={marker:match[1],start:match.index!+match[0].length+(answer[match.index!+match[0].length]==='\n'?1:0),language:/^[a-z0-9_+#.-]{0,40}$/.test(language)?language:'plaintext',path};
  } else if(match[1][0]===open.marker[0]&&match[1].length>=open.marker.length&&!match[2].trim()) {
   const code=answer.slice(open.start,match.index);
   if(code.length&&code.length<=16_384)samples.push({code,language:open.language,...(open.path?{path:open.path}:{})});
   open=undefined;if(samples.length===16)break;
  }
 }
 return samples;
}
