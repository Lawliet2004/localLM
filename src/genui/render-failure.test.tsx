import { describe,it,expect,vi } from 'vitest';
import { render,screen,waitFor } from '@testing-library/react';
import VisualAnswer from './VisualAnswer';
import { api } from '../lib/api';
import type { PresentationRecord } from '../lib/types';
vi.mock('../lib/api',()=>({nativeAvailable:false,api:{saveVisualState:vi.fn(async()=>{}),rejectPresentation:vi.fn(async()=>{})}}));
vi.mock('@tauri-apps/plugin-opener',()=>({openUrl:vi.fn()}));
vi.mock('@openuidev/react-lang',async original=>({...await original<typeof import('@openuidev/react-lang')>(),Renderer:()=>{throw new Error('component failure with arbitrary diagnostic payload');}}));
describe('render failure persistence',()=>{
 it('preserves the answer and reports only owned program identity on a renderer exception',async()=>{
  const log=vi.spyOn(console,'error').mockImplementation(()=>{});
  const p:PresentationRecord={messageId:'owned',format:'openui',formatVersion:1,libraryVersion:'locallm.openui/2',sourceText:'Original answer',uiProgram:'root = Response([Text("Visual")])',interactiveState:{},generationStatus:'complete'};
  render(<VisualAnswer record={p} conversationId="conversation"/>);
  await waitFor(()=>expect(api.rejectPresentation).toHaveBeenCalledWith('conversation','owned',p.uiProgram));
  expect(screen.getByText('Original answer',{selector:'p'})).toBeInTheDocument();
  expect(screen.queryByText('component failure with arbitrary diagnostic payload')).toBeNull();log.mockRestore();
 });
});
