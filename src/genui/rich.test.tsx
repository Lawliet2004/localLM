import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { ValidatedRenderer } from './ValidatedRenderer';
import { EvidenceContext } from './grounding';
import { binarySearch, lru, sorting, traversal,sqlJoin } from './simulations';
import { validateProgram } from './validation';
vi.mock('@tauri-apps/plugin-opener',()=>({openUrl:vi.fn()}));
const program=(body:string)=>`root = Response([${body}])`;
describe('reusable visual components',()=>{
 it('renders exact chart values and an accessible diagram',()=>{
  render(<ValidatedRenderer response={program('Chart("Sales", "bar", [{label:"Jan",value:10},{label:"Feb",value:15},{label:"Mar",value:25}]), Diagram("TCP", [{id:"a",label:"SYN"},{id:"b",label:"SYN-ACK"},{id:"c",label:"ACK"}], [{from:"a",to:"b"},{from:"b",to:"c"}])')}/>);
  expect(screen.getByRole('img',{name:'bar chart: Sales'})).toBeInTheDocument();
  expect(screen.getAllByRole('cell').map(n=>n.textContent)).toEqual(['10','15','25']);
  expect(screen.getAllByText('SYN')[0]).toBeInTheDocument();expect(screen.getAllByText('SYN-ACK')[0]).toBeInTheDocument();expect(screen.getAllByText('ACK')[0]).toBeInTheDocument();
 });
 it('scores a quiz locally and restores through the official state callback',()=>{
  const source=program('Quiz("joins", "SQL joins", [{question:"Keeps unmatched left rows?",options:["Inner","Left"],answer:1,explanation:"LEFT JOIN keeps all left rows."}])');
  const saved=vi.fn();const first=render(<ValidatedRenderer response={source} onStateUpdate={saved}/>);
  fireEvent.click(screen.getByRole('radio',{name:'Left'}));expect(screen.getByText('Score: 1 / 1')).toBeInTheDocument();
  const state=saved.mock.calls.slice(-1)[0]?.[0];expect(state).toBeDefined();first.unmount();
  render(<ValidatedRenderer response={source} initialState={state}/>);expect(screen.getByRole('radio',{name:'Left'})).toBeChecked();
 });
 it('advances binary search without a model request',()=>{
  render(<ValidatedRenderer response={program('InteractiveExample("search", "binary-search", [2,4,6,8,10], 8)')}/>);
  expect(screen.getByText(/Range 0–4/)).toBeInTheDocument();fireEvent.click(screen.getByRole('button',{name:'Next step'}));expect(screen.getByText(/Range 3–4/)).toBeInTheDocument();
 });
 it('flips flashcards and updates local controls',()=>{
  render(<ValidatedRenderer response={program('Flashcard("card", "Question", "Answer"), Toggle("toggle", "Show", false), Slider("rate", "Rate", 0, 10, 3)')}/>);
  fireEvent.click(screen.getByRole('button',{name:'Show answer'}));expect(screen.getByText('Answer')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('checkbox'));expect(screen.getByRole('checkbox')).toBeChecked();
  fireEvent.change(screen.getByRole('slider'),{target:{value:'7'}});expect(screen.getByRole('slider')).toHaveValue('7');
 });
 it('only turns actual evidence IDs into citation links',()=>{
  render(<EvidenceContext.Provider value={{s1:'https://example.com/evidence'}}><ValidatedRenderer response={program('SourceCard("s1", "Known"), SourceCard("invented", "Fake")')}/></EvidenceContext.Provider>);
  expect(screen.getByRole('link',{name:'[s1] Known'})).toHaveAttribute('href','https://example.com/evidence');expect(screen.queryByRole('link',{name:'Fake'})).not.toBeInTheDocument();
 });
 it('rejects invalid chart and graph configs before render',()=>{
  for(const body of ['Chart("Pie", "pie", [{label:"Bad",value:-1}])','Diagram("Bad",[{id:"a",label:"A"}],[{from:"a",to:"missing"}])','Quiz("bad","Bad",[{question:"Q",options:["A","B"],answer:5,explanation:"E"}])'])expect(validateProgram(program(body)).valid).toBe(false);
 });
});
describe('deterministic simulation algorithms',()=>{
 it('uses SQL bag semantics including unmatched outer rows',()=>{expect(sqlJoin([1,1],[1,1],'inner')).toHaveLength(4);expect(sqlJoin([1],[2],'full')).toEqual([[1,null],[null,2]]);});
 it('finds present targets and reports absent ones',()=>{expect(binarySearch([9,1,7,3,5],7).slice(-1)[0]?.message).toContain('Found 7');expect(binarySearch([1,3,5],4).slice(-1)[0]?.message).toContain('not found');});
 it('sorts input immutably',()=>{const input=[3,1,2];expect(sorting(input).slice(-1)[0]?.values).toEqual([1,2,3]);expect(input).toEqual([3,1,2]);});
 it('evicts the least recently used entry after a hit',()=>{expect(lru([1,2,1,3],2).slice(-1)[0]?.values).toEqual([3,1]);});
 it('traverses cycles without revisits',()=>{expect(traversal([10,20,30],[[0,1],[1,0],[0,2]],false).map(frame=>frame.values.slice(-1)[0])).toEqual([10,20,30]);});
});
