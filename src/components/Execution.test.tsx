import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Execution } from './Execution';
const fixtures=vi.hoisted(()=>({
  pending:vi.fn(),
  execution:vi.fn(async()=>({pythonPath:'',nodePath:'',powershellPath:''})),
  search:vi.fn(async()=>({provider:'searxng',searxngBaseUrl:'http://127.0.0.1:8080',hasGoogleApiKey:false,googleCxId:'',searchFallbackEnabled:true})),
  hasKey:vi.fn(async()=>false),
  testInterpreter:vi.fn(),
  open:vi.fn(),
  isolationGet:vi.fn(async()=>({mode:'unsandboxed',network:false,lan:false,lpac:false,toolchainDirs:[],processLimit:32,memoryMb:2048})),
  isolationStatus:vi.fn(async()=>({available:true,kind:null,reason:'AppContainer isolation is available.',policy:{mode:'unsandboxed',network:false,lan:false,lpac:false,toolchainDirs:[],processLimit:32,memoryMb:2048},grants:[]})),
  isolationSave:vi.fn(async()=>{}),
}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:fixtures.open}));
vi.mock('../lib/api',()=>({nativeAvailable:true,errorMessage:String,api:{
  getExecutionConfig:fixtures.execution,
  getWebSearchConfig:fixtures.search,
  pendingDaytonaOperations:fixtures.pending,
  hasDaytonaKey:fixtures.hasKey,
  testInterpreter:fixtures.testInterpreter,
  isolationGet:fixtures.isolationGet,
  isolationStatus:fixtures.isolationStatus,
  isolationSave:fixtures.isolationSave,
}}));
describe('cloud recovery visibility',()=>{
  it('clears interpreter verification after the selected executable changes',async()=>{
    fixtures.execution.mockResolvedValueOnce({pythonPath:'C:/old.exe',nodePath:'',powershellPath:''});
    fixtures.pending.mockReset().mockResolvedValue([]);
    fixtures.testInterpreter.mockResolvedValueOnce('Python 3.12 verified');
    fixtures.open.mockResolvedValueOnce('C:/new.exe');
    render(<Execution/>);
    const input=await screen.findByRole('textbox',{name:'Python executable'});
    await vi.waitFor(()=>expect(input).toHaveValue('C:/old.exe'));
    fireEvent.click(screen.getAllByRole('button',{name:'Test'})[0]);
    expect(await screen.findByText('Python 3.12 verified')).toBeVisible();
    fireEvent.click(screen.getAllByRole('button',{name:'Browse'})[0]);
    await vi.waitFor(()=>expect(input).toHaveValue('C:/new.exe'));
    expect(screen.queryByText('Python 3.12 verified')).not.toBeInTheDocument();
    expect(screen.queryByRole('button',{name:'Testing…'})).not.toBeInTheDocument();
    fixtures.testInterpreter.mockResolvedValueOnce('Python 3.13 verified');
    fireEvent.click(screen.getAllByRole('button',{name:'Test'})[0]);
    expect(await screen.findByText('Python 3.13 verified')).toBeVisible();
    fireEvent.change(input,{target:{value:'C:/manual.exe'}});
    expect(screen.queryByText('Python 3.13 verified')).not.toBeInTheDocument();
  });
  it('loads only the visible settings section without background cloud polling',async()=>{
    vi.useFakeTimers();
    fixtures.pending.mockReset().mockResolvedValue([]);
    fixtures.execution.mockClear(); fixtures.search.mockClear(); fixtures.hasKey.mockClear();
    const view=render(<Execution section="web-search"/>);
    try {
      await act(async()=>{await vi.advanceTimersByTimeAsync(15000);});
      expect(fixtures.search).toHaveBeenCalledTimes(1);
      expect(fixtures.execution).not.toHaveBeenCalled();
      expect(fixtures.hasKey).not.toHaveBeenCalled();
      expect(fixtures.pending).not.toHaveBeenCalled();
      view.rerender(<Execution section="execution"/>);
      await act(async()=>{});
      expect(fixtures.execution).toHaveBeenCalledTimes(1);
      expect(fixtures.pending).toHaveBeenCalledTimes(1);
      expect(fixtures.search).toHaveBeenCalledTimes(1);
    } finally { view.unmount(); vi.useRealTimers(); }
  });
  it('offers local execution without credentials for an unavailable cloud feature',async()=>{
    fixtures.pending.mockReset().mockResolvedValue([]);
    render(<Execution/>);
    expect(await screen.findByRole('button',{name:'Save local execution'})).toBeInTheDocument();
    expect(screen.queryByLabelText('Daytona API key')).not.toBeInTheDocument();
  });
  it('refreshes background cleanup and stops polling after unmount',async()=>{
    vi.useFakeTimers();
    fixtures.pending.mockReset().mockResolvedValueOnce([{name:'locallm-first',sandboxId:'sandbox-1',createdAt:1,cleanupError:null}]).mockResolvedValue([]);
    const view=render(<Execution/>);
    try {
      await act(async()=>{});
      expect(screen.getByRole('region',{name:'Pending cloud cleanup'})).toBeInTheDocument();
      await act(async()=>{await vi.advanceTimersByTimeAsync(5000);});
      expect(screen.queryByRole('region',{name:'Pending cloud cleanup'})).not.toBeInTheDocument();
      expect(fixtures.pending).toHaveBeenCalledTimes(2);
      view.unmount();
      await act(async()=>{await vi.advanceTimersByTimeAsync(10000);});
      expect(fixtures.pending).toHaveBeenCalledTimes(2);
    } finally { view.unmount(); vi.useRealTimers(); }
  });
  it('shows unknown creation outcomes and retained cleanup failures',async()=>{
    fixtures.pending.mockResolvedValue([{name:'locallm-first',sandboxId:null,createdAt:1,cleanupError:null},{name:'locallm-second',sandboxId:'sandbox-2',createdAt:2,cleanupError:'Daytona returned HTTP 503.'}]);
    render(<Execution/>);
    expect(await screen.findByRole('region',{name:'Pending cloud cleanup'})).toBeInTheDocument();
    expect(screen.getByText(/Creation outcome unknown/)).toBeInTheDocument();
    expect(screen.getByText('Sandbox: sandbox-2')).toBeInTheDocument();
    expect(screen.getByText('Daytona returned HTTP 503.')).toBeInTheDocument();
  });
  it('reports journal failures instead of claiming cleanup is clear',async()=>{
    fixtures.pending.mockRejectedValue(new Error('Journal unavailable'));
    render(<Execution/>);
    expect(await screen.findByRole('alert')).toHaveTextContent('Journal unavailable');
  });
  it('shows enforced-only sandbox controls after the mode changes',async()=>{
    fixtures.pending.mockReset().mockResolvedValue([]);
    render(<Execution/>);
    const select=await screen.findByRole('combobox',{name:'Isolation mode'});
    expect(screen.queryByRole('checkbox',{name:'Allow network in the sandbox'})).not.toBeInTheDocument();
    fireEvent.change(select,{target:{value:'enforced'}});
    expect(await screen.findByRole('checkbox',{name:'Allow network in the sandbox'})).toBeInTheDocument();
    fireEvent.change(screen.getByRole('combobox',{name:'Isolation mode'}),{target:{value:'unsandboxed'}});
    expect(screen.queryByRole('checkbox',{name:'Allow network in the sandbox'})).not.toBeInTheDocument();
  });
  it('shows the unavailable reason and disables enforced when the container cannot be created',async()=>{
    fixtures.pending.mockReset().mockResolvedValue([]);
    fixtures.isolationStatus.mockResolvedValueOnce({available:false,kind:null,reason:'AppContainer is unavailable on this system; enforced mode will refuse to run.',policy:{mode:'unsandboxed',network:false,lan:false,lpac:false,toolchainDirs:[],processLimit:32,memoryMb:2048},grants:[]});
    render(<Execution/>);
    expect(await screen.findByText(/AppContainer is unavailable/)).toBeInTheDocument();
    expect(await screen.findByRole('combobox',{name:'Isolation mode'})).toBeDisabled();
  });
});
