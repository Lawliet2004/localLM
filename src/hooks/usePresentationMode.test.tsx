import { act,renderHook,waitFor } from '@testing-library/react';
import { describe,expect,it,vi } from 'vitest';
import { usePresentationMode, PRESENTATION_DEFAULT_CHANGED } from './usePresentationMode';
import { api } from '../lib/api';
vi.mock('../lib/api',()=>({api:{getPresentationMode:vi.fn(async()=>({defaultMode:'auto',overrideMode:null,effectiveMode:'auto'}))}}));
describe('presentation settings ownership',()=>{
 it('does not apply a pending read to a different conversation',async()=>{
  let finish!:(v:unknown)=>void;vi.mocked(api.getPresentationMode).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve as typeof finish;}));
  const {result,rerender}=renderHook(({id})=>usePresentationMode(id),{initialProps:{id:'A'}});
  rerender({id:'B'});await act(async()=>{finish({defaultMode:'text',overrideMode:'visual',effectiveMode:'visual'});});
  expect(result.current.effectiveMode).toBe('auto');
 });
 it('refreshes the same mounted conversation and protects its saved override from a slow read',async()=>{
  let finish!:(v:unknown)=>void;vi.mocked(api.getPresentationMode).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve as typeof finish;}));
  const {result}=renderHook(()=>usePresentationMode('owned'));
  act(()=>window.dispatchEvent(new CustomEvent(PRESENTATION_DEFAULT_CHANGED,{detail:'visual'})));
  expect(result.current.effectiveMode).toBe('visual');
  await act(async()=>finish({defaultMode:'auto',overrideMode:'text',effectiveMode:'text'}));
  expect(result.current.defaultMode).toBe('visual');expect(result.current.effectiveMode).toBe('text');
 });
 it('uses the global default for a new conversation without saving an override',async()=>{
  const {result}=renderHook(()=>usePresentationMode('new'));await waitFor(()=>expect(api.getPresentationMode).toHaveBeenCalledWith(null));
  act(()=>window.dispatchEvent(new CustomEvent(PRESENTATION_DEFAULT_CHANGED,{detail:'text'})));
  expect(result.current.effectiveMode).toBe('text');expect(result.current.overrideMode).toBeNull();
 });
});
