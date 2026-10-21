import { act,fireEvent,render,screen } from '@testing-library/react';
import { describe,it,expect,vi,afterEach } from 'vitest';
import VisualAnswer from './VisualAnswer';
import type { PresentationRecord } from '../lib/types';
import { api } from '../lib/api';
vi.mock('../lib/api',()=>({nativeAvailable:false,api:{saveVisualState:vi.fn(async()=>{}),rejectPresentation:vi.fn(async()=>{})}}));
vi.mock('@tauri-apps/plugin-opener',()=>({openUrl:vi.fn()}));
const base:PresentationRecord={messageId:'m',format:'openui',formatVersion:1,libraryVersion:'locallm.openui/2',sourceText:'The factual answer is preserved.',uiProgram:'root = Response([Text("Visual facts")])',interactiveState:{},generationStatus:'complete'};
afterEach(()=>vi.useRealTimers());
describe('persisted visual answer boundary',()=>{
 it.each(['error','interrupted'] as const)('preserves original for %s programs',status=>{render(<VisualAnswer record={{...base,generationStatus:status}} conversationId="c"/>);expect(screen.getByText(base.sourceText)).toBeInTheDocument();});
 it('falls back for malformed programs and incompatible library versions',()=>{const {rerender}=render(<VisualAnswer record={{...base,uiProgram:'bad'}} conversationId="c"/>);expect(screen.getByText(base.sourceText)).toBeInTheDocument();rerender(<VisualAnswer record={{...base,libraryVersion:'future/99'}} conversationId="c"/>);expect(screen.getByText(base.sourceText)).toBeInTheDocument();});
 it('switches to the original answer and accepts non-map stored source metadata safely',()=>{render(<VisualAnswer record={base} conversationId="c" sourceLinks="null"/>);expect(screen.getByText('Visual facts')).toBeInTheDocument();fireEvent.click(screen.getByRole('button',{name:'Original answer'}));expect(screen.getByText(base.sourceText)).toBeInTheDocument();});
 it('repairs supported syntax locally and rejects invented link actions',()=>{render(<VisualAnswer record={{...base,uiProgram:'root = Response(children: [Text(text:"Facts"),Button("Source",{type:"open_url",url:"https://attacker.example"})])'}} conversationId="c"/>);expect(screen.getByText('Facts')).toBeInTheDocument();expect(screen.getByRole('button',{name:'Source'})).toBeDisabled();});
 it('renders a real native-model chart after syntax-only closer repair',()=>{
 render(<VisualAnswer record={{...base,sourceText:'January sold 10 units. February sold 15 units. March sold 25 units.',uiProgram:'root = Response([Chart(title:"Sales Data",kind:"bar",points:[{label:"January",value:10},{label:"February",value:15},{label:"March",value:25}]])'}} conversationId="c"/>);
 expect(screen.getByRole('combobox',{name:'Chart type'})).toBeInTheDocument();expect(screen.getByText('Component argument syntax corrected locally.')).toBeInTheDocument();
 });
 it('keeps grounded committed content when a later streamed child invents data',()=>{
 const {rerender}=render(<VisualAnswer record={{...base,generationStatus:'streaming',uiProgram:'root = Response([Text("First")'}} conversationId="c"/>);
 const first=screen.getByText('First');
 rerender(<VisualAnswer record={{...base,generationStatus:'streaming',uiProgram:'root = Response([Text("First"),Chart("Fake","bar",[{label:"Jan",value:999}])])'}} conversationId="c"/>);
 expect(screen.getByText('First')).toBe(first);expect(screen.queryByRole('img')).toBeNull();
 });
 it('debounces state writes with message and conversation identity',async()=>{vi.useFakeTimers();render(<VisualAnswer record={{...base,uiProgram:'root = Response([Toggle("a", "Enabled", false)])'}} conversationId="owned"/>);fireEvent.click(screen.getByRole('checkbox'));await act(async()=>{vi.advanceTimersByTime(300);});expect(api.saveVisualState).toHaveBeenCalledWith('owned','m',expect.any(Object));});
 it('routes conversation actions through the visible follow-up callback',()=>{const follow=vi.fn();render(<VisualAnswer record={{...base,uiProgram:'root = Response([Button("Explain further")])'}} conversationId="c" onSendPrompt={follow}/>);fireEvent.click(screen.getByRole('button',{name:'Explain further'}));expect(follow).toHaveBeenCalledWith('Explain further');});
});
