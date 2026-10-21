import {describe,it,expect,vi} from 'vitest';
import {render,screen} from '@testing-library/react';
import {numericTables,codeSamples} from './authoritativeData';
import {validateChartGrounding} from './dataGrounding';
import VisualAnswer from './VisualAnswer';
import type {PresentationRecord} from '../lib/types';
vi.mock('../lib/api',()=>({nativeAvailable:false,api:{rejectPresentation:vi.fn(async()=>{}),saveVisualState:vi.fn(async()=>{})}}));
vi.mock('@tauri-apps/plugin-opener',()=>({openUrl:vi.fn()}));
const answer='| Month | Units Sold |\n| --- | --- |\n| January | 10 |\n| February | 15 |\n| March | 25 |';
describe('host-bound numeric facts',()=>{
 it('preserves exact labels, units and values from typed tables and rejects ambiguous data',()=>{
  expect(numericTables('* January: 10 units\n* February: 15 units\n* March: 25 units')[0]).toEqual({columns:['Label','units'],points:[{label:'January',value:10},{label:'February',value:15},{label:'March',value:25}]});
  expect(numericTables(answer)).toEqual([{columns:['Month','Units Sold'],points:[{label:'January',value:10},{label:'February',value:15},{label:'March',value:25}]}]);
  expect(numericTables(answer.replace('| 15 |','| $15 |'))).toEqual([]);
  expect(numericTables('| A | B | C |\n| --- | --- | --- |\n| x | 1 | 2 |')).toEqual([]);
  expect(numericTables('| A | B |\n| --- | --- |\n'+Array.from({length:101},(_,i)=>`| Item ${i} | ${i} |`).join('\n'))).toEqual([]);
 });
 it('keeps original code bytes and ignores incomplete fences without executing anything',()=>{
  const source='lru.py\n```python\nprint("<script>never execute</script>")\n```\n';
  expect(codeSamples(source)).toEqual([{language:'python',code:'print("<script>never execute</script>")\n',path:'lru.py'}]);
  expect(codeSamples('```python\npartial')).toEqual([]);
  expect(codeSamples('This is not code from b.py:\n```python\nprint(1)\n```')[0].path).toBeUndefined();
  const record:PresentationRecord={messageId:'code',format:'openui',formatVersion:1,libraryVersion:'locallm.openui/2',sourceText:source,uiProgram:'root = Response([SourceCode(0,"lru.py")])',interactiveState:{},generationStatus:'complete'};
  render(<VisualAnswer record={record} conversationId="c"/>);
  expect(screen.getByRole('button',{name:'Copy code block'})).toBeInTheDocument();
  expect(document.querySelector('script')).toBeNull();
  expect(validateChartGrounding(record.uiProgram,source)).toBe(true);
  const two='a.py\n```python\nprint(1)\n```\nb.py\n```python\nprint(2)\n```';
  expect(validateChartGrounding('root = Response([SourceCode(0,"b.py")])',two)).toBe(false);
  expect(validateChartGrounding('root = Response([FilePreview("b.py","print(1)","python")])',two)).toBe(false);
  expect(validateChartGrounding('root = Response([SourceCode(0,"invented.py")])',source)).toBe(false);
 });
 it('renders facts through the official renderer without model numeric transcription',()=>{
  const record:PresentationRecord={messageId:'m',format:'openui',formatVersion:1,libraryVersion:'locallm.openui/2',sourceText:answer,uiProgram:'root = Response([SourceChart("chart",0,"bar")])',interactiveState:{},generationStatus:'complete'};
  render(<VisualAnswer record={record} conversationId="c"/>);
  expect(screen.getAllByRole('cell').map(cell=>cell.textContent)).toEqual(['10','15','25']);
  expect(screen.getByRole('img',{name:'bar chart: Units Sold'})).toBeInTheDocument();
  expect(validateChartGrounding(record.uiProgram,answer)).toBe(true);
  expect(validateChartGrounding('root = Response([SourceChart("chart",7,"bar")])',answer)).toBe(false);
  expect(validateChartGrounding('root = Response([SourceChart("chart",0,"scatter")])',answer)).toBe(false);
 });
});
