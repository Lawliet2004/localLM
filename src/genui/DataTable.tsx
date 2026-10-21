import { useStateField,useIsStreaming } from '@openuidev/react-lang';
import type { z } from 'zod/v4';
import { Table } from './catalog';
export function DataTable({props:p}:{props:z.infer<typeof Table.props>}){
 const field=useStateField<{column:number;descending:boolean}>(p.id??`table-${p.columns.join('-')}`,{column:-1,descending:false});const streaming=useIsStreaming();
 const sort=field.value&&typeof field.value==='object'?field.value:{column:-1,descending:false};
 const rows=[...p.rows];if(Number.isInteger(sort.column)&&sort.column>=0&&sort.column<p.columns.length)rows.sort((a,b)=>{const x=a[sort.column],y=b[sort.column];return(typeof x==='number'&&typeof y==='number'?x-y:String(x).localeCompare(String(y)))*(sort.descending?-1:1);});
 return <div className="genui-table-wrap" role="region" aria-label="Data table" tabIndex={0}><table className="genui-table"><thead><tr>{p.columns.map((column,i)=><th key={i} aria-sort={sort.column===i?(sort.descending?'descending':'ascending'):'none'}><button type="button" className="genui-table-sort" disabled={streaming} onClick={()=>field.setValue({column:i,descending:sort.column===i?!sort.descending:false})} aria-label={`Sort by ${column}`}>{column}{sort.column===i?(sort.descending?' ↓':' ↑'):''}</button></th>)}</tr></thead><tbody>{rows.map((row,i)=><tr key={i}>{row.map((cell,j)=><td key={j}>{cell}</td>)}</tr>)}</tbody></table></div>;
}
