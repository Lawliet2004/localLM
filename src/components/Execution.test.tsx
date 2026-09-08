import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { Execution } from './Execution';
const fixtures=vi.hoisted(()=>({pending:vi.fn()}));
vi.mock('../lib/api',()=>({nativeAvailable:true,errorMessage:String,api:{
  getExecutionConfig:async()=>({pythonPath:'',nodePath:'',powershellPath:''}),
  pendingDaytonaOperations:fixtures.pending,
  hasDaytonaKey:async()=>false,
}}));
describe('cloud recovery visibility',()=>{
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
});
