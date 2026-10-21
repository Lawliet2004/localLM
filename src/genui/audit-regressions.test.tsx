import { describe, it, expect, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { ValidatedRenderer } from './ValidatedRenderer';
import { validateProgram } from './validation';
import { validateChartGrounding } from './dataGrounding';
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
const quiz = 'Quiz("joins","SQL",[{question:"Join?",options:["Inner","Left"],answer:1,explanation:"Keeps left."}])';
describe('audit correctness regressions', () => {
  it('rejects swapped facts, invented rows/metrics/files and unsupported citations', () => {
    const source = 'Jan sales: 10; Feb sales: 15.';
    for (const child of ['Chart("Sales","bar",[{label:"Jan",value:15},{label:"Feb",value:10}])','Table(["Month","Sales"],[["Jan",999999]])','Table(["Month","Revenue USD"],[["Jan",10]])','Chart("Temperature Kelvin","bar",[{label:"Jan",value:10}])','Stat("Jan revenue","999999 dollars")','FilePreview("secret.py","print(999)","python")','SourceCard("s1","Invented claim")']) expect(validateChartGrounding(`root = Response([${child}])`, source, {s1:'https://example.com'})).toBe(false);
    expect(validateChartGrounding('root = Response([Chart("Sales","bar",[{label:"Jan",value:10},{label:"Feb",value:15}])])', source)).toBe(true);
    expect(validateChartGrounding('root = Response([Chart("Revenue USD","bar",[{label:"Jan",value:10}])])', source)).toBe(false);
  });
  it('retains the same visible node through partial Chart and Quiz children, including named output', () => {
    const {rerender} = render(<ValidatedRenderer response={'root = Response([Text("First")'} isStreaming />);
    const first = screen.getByText('First');
    for (const suffix of [', Chart("Sales","bar",[', ', '+quiz.slice(0,30), ', Chart(title:"Sales",kind:"bar",points:[']) {
      rerender(<ValidatedRenderer response={'root = Response([Text("First")'+suffix} isStreaming />);
      expect(screen.getByText('First')).toBe(first);
    }
    rerender(<ValidatedRenderer response={'root = Response(children:[Text(text:"First"),'+quiz+'])'} isStreaming />);
    expect(screen.getByText('First')).toBe(first);
    expect(screen.getAllByRole('radio')[0]).toBeDisabled();
  });
  it('commits root-first references while later referenced children remain incomplete', () => {
    const {rerender}=render(<ValidatedRenderer response={'root = Response([a,b])\na = Text("First")'} isStreaming/>);
    const first=screen.getByText('First');
    rerender(<ValidatedRenderer response={'root = Response([a,b])\na = Text("First")\nb = Chart("Sales","bar",['} isStreaming/>);
    expect(screen.getByText('First')).toBe(first);
  });
  it('isolates repeated quiz IDs across visible messages and restores each independently', () => {
    const program = `root = Response([${quiz}])`;
    const savedA=vi.fn(),savedB=vi.fn();
    const {rerender} = render(<><ValidatedRenderer response={program} onStateUpdate={savedA}/><ValidatedRenderer response={program} onStateUpdate={savedB}/></>);
    const radios = screen.getAllByRole('radio');
    fireEvent.click(radios[1]); fireEvent.click(radios[3]);
    expect(radios[1]).toBeChecked(); expect(radios[3]).toBeChecked();
    fireEvent.click(radios[2]);
    const stateA=savedA.mock.lastCall?.[0],stateB=savedB.mock.lastCall?.[0];
    expect(stateA.joins.value[0]).toBe(1);expect(stateB.joins.value[0]).toBe(0);
    rerender(<><ValidatedRenderer key="a" response={program} initialState={stateA}/><ValidatedRenderer key="b" response={program} initialState={stateB}/></>);
    expect(screen.getAllByRole('radio')[1]).toBeChecked(); expect(screen.getAllByRole('radio')[2]).toBeChecked();
    expect(validateProgram(`root = Response([${quiz},${quiz}])`).valid).toBe(false);
    expect(validateProgram('root = Response([Calculator("a","Calc","add",1,2),Slider("a-a","Collision",0,10,1)])').valid).toBe(false);
  });
  it.each([101,-101,1024])('computes the requested exponent %s or reports overflow', exponent => {
    render(<ValidatedRenderer response={`root = Response([Calculator("c","Power","power",2,${exponent})])`}/>);
    const value = 2 ** exponent;
    expect(screen.getByText(`power: ${Number.isFinite(value)?String(value):'Outside the finite numeric range'}`)).toBeInTheDocument();
  });
  it.each(['bar','line','pie','donut','scatter'])('renders finite accessible %s coordinates for constant and zero ranges', kind => {
    const points = ['pie','donut'].includes(kind) ? '[{label:"Very long category label with spaces",value:1}]' : '[{label:"Zero",x:0,value:0},{label:"Negative",x:0,value:-2}]';
    const {container} = render(<ValidatedRenderer response={`root = Response([Chart("Bounds","${kind}",${points})])`}/>);
    expect(container.innerHTML).not.toMatch(/NaN|Infinity/);
    expect(screen.getByText('Chart values and legend')).toBeInTheDocument();
  });
  it('preserves an out-of-policy operand and reports it without silently clamping', () => {
    render(<ValidatedRenderer response='root = Response([Calculator("c","Power","power",2,101)])'/>);
    const second=screen.getByRole('spinbutton',{name:'Second value'});
    fireEvent.change(second,{target:{value:'1000000000001'}});
    expect(second).toHaveValue(1000000000001);
    expect(screen.getByText(/Enter finite operands/)).toBeInTheDocument();
  });
  it('reports undefined arithmetic domains' , () => {
    render(<ValidatedRenderer response='root = Response([Calculator("c","Power","power",-2,0.5)])'/>);
    expect(screen.getByText('power: Undefined for these operands')).toBeInTheDocument();
  });
  it('scales scatter numeric ticks and exposes both coordinates', () => {
    const {container} = render(<ValidatedRenderer response='root = Response([Chart("Points","scatter",[{label:"A",x:0,value:-1},{label:"B",x:1,value:2},{label:"C",x:100,value:2}])])'/>);
    expect([...container.querySelectorAll('circle')].map(n=>Number(n.getAttribute('cx')))).toEqual([40,45.2,560]);
    expect([...container.querySelectorAll('text[text-anchor="middle"]')].map(n=>[n.textContent,Number(n.getAttribute('x'))])).toEqual([['0',40],['50',300],['100',560]]);
    expect(screen.getByRole('columnheader',{name:'X'})).toBeInTheDocument();
    expect(screen.getByRole('columnheader',{name:'Y'})).toBeInTheDocument();
    expect(container.querySelectorAll('circle title')[1]).toHaveTextContent('B: x=1, y=2');
  });
});
