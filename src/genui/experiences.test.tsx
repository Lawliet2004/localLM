import { describe,it,expect,vi } from 'vitest';
import { fireEvent,render,screen,within } from '@testing-library/react';
import { ValidatedRenderer } from './ValidatedRenderer';
import { validateProgram } from './validation';
import { alignedDiff } from './diff';
import { EvidenceContext,ResearchContext } from './grounding';
vi.mock('@tauri-apps/plugin-opener',()=>({openUrl:vi.fn()}));
describe('bounded richer experiences',()=>{
 it('lays out directed nodes and retains full accessible labels',()=>{
  const {container}=render(<ValidatedRenderer response='root = Response([Diagram("TCP",[{id:"a",label:"SYN"},{id:"b",label:"SYN-ACK"},{id:"c",label:"ACK"}],[{from:"a",to:"b"},{from:"b",to:"c"}])])'/>);
  const ys=[...container.querySelectorAll('rect')].map(n=>Number(n.getAttribute('y')));
  expect(ys).toEqual([20,140,260]);expect(container.querySelectorAll('path[marker-end]')).toHaveLength(2);
  expect(screen.getByText('Nodes and connections')).toBeInTheDocument();
 });
 it('groups read-only paths into collapsible directories without host actions',()=>{
  render(<ValidatedRenderer response='root = Response([FileTree(["src/a.ts","src/nested/b.ts","README.md"])])'/>);
  const hierarchy=screen.getByRole('navigation',{name:'Read-only file hierarchy'});
  expect(within(hierarchy).getByText('src/').closest('details')).toHaveAttribute('open');
  expect(within(hierarchy).getByText('nested/').closest('details')?.parentElement?.closest('details')).toHaveAttribute('open');
  fireEvent.click(within(hierarchy).getByText('src/'));expect(within(hierarchy).queryByRole('button')).toBeNull();
 });
 it('aligns replacements and insertion gaps across multiple hunks',()=>{
  const rows=alignedDiff('--- a/file\n+++ b/file\n@@ -3,3 +3,4 @@\n same\n-old\n+new\n+extra\n tail\n@@ -20 +21 @@\n-last\n+final');
  expect(rows.filter(row=>!row.heading)).toEqual([{before:'same',after:'same',oldLine:3,newLine:3},{before:'old',after:'new',oldLine:4,newLine:4},{before:undefined,after:'extra',oldLine:undefined,newLine:5},{before:'tail',after:'tail',oldLine:5,newLine:6},{before:'last',after:'final',oldLine:20,newLine:21}]);
 });
 it('treats header-looking lines inside counted hunks as file content',()=>{
  expect(alignedDiff('--- a/file\n+++ b/file\n@@ -1 +1 @@\n--- a\n+++ b').filter(row=>!row.heading)).toEqual([{before:'-- a',after:'++ b',oldLine:1,newLine:1}]);
  expect(alignedDiff('@@ -99999999999999999999 +1 @@\n-old\n+new').some(row=>row.oldLine!==undefined)).toBe(false);
 });
 it('binds a typed slider to local mathematical data and rejects absent bindings',()=>{
  const source='root = Response([Slider("slope","Slope",-2,2,1),FunctionPlot("Linear","slope","linear",-2,2)])';
  const {container}=render(<ValidatedRenderer response={source}/>);
  const before=container.querySelector('polyline')?.getAttribute('points');
  fireEvent.change(screen.getByRole('slider',{name:'Slope'}),{target:{value:'-2'}});
  expect(container.querySelector('polyline')?.getAttribute('points')).not.toBe(before);
  expect(screen.getByText(/a = -2/)).toBeInTheDocument();
  expect(validateProgram('root = Response([FunctionPlot("Bad","missing","linear",0,1)])').valid).toBe(false);
 });
 it('uses the linked slider default in either component order without inventing a value during streaming',()=>{
  const source='root = Response([FunctionPlot("Linear","slope","linear",-2,2),Slider("slope","Slope",-3,3,2)])';
  const {rerender}=render(<ValidatedRenderer response={source} isStreaming/>);
  expect(screen.queryByRole('img')).toBeNull();
  rerender(<ValidatedRenderer response={source}/>);
  expect(screen.getByText(/a = 2/)).toBeInTheDocument();expect(screen.getByRole('slider',{name:'Slope'})).toHaveValue('2');
 });
 it('uses actual host evidence titles and claim verdicts',()=>{
  render(<EvidenceContext.Provider value={{s1:'https://example.com/source'}}><ResearchContext.Provider value={{sources:[{url:'https://example.com/source',ids:['s1'],title:'Collected title',snippet:'Actual excerpt'}],claims:[{claim:'Host claim',verdict:'UNSUPPORTED',citations:['s1','unknown']}]}}><ValidatedRenderer response='root = Response([EvidenceView(["s1"])])'/></ResearchContext.Provider></EvidenceContext.Provider>);
  expect(screen.getByRole('link')).toHaveTextContent('Collected title');expect(screen.getByText('UNSUPPORTED')).toBeInTheDocument();expect(screen.getByText('Actual excerpt')).toBeInTheDocument();expect(screen.queryByText('unknown')).toBeNull();
 });
 it('toggles supplied series locally',()=>{
  const {container}=render(<ValidatedRenderer response='root = Response([SeriesChart("Comparison",[{name:"A",points:[{label:"Jan",value:10}]},{name:"B",points:[{label:"Jan",value:15}]}])])'/>);
  expect([...container.querySelectorAll('polyline')].map(line=>line.getAttribute('points'))).toEqual(['40,90','40,30']);
  expect(screen.getAllByRole('img')).toHaveLength(2);fireEvent.click(screen.getByRole('checkbox',{name:'A'}));expect(screen.getAllByRole('img')).toHaveLength(1);
 });
});
