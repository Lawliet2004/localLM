import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe,it,expect,vi } from 'vitest';
import { PresentationDefaults } from './PresentationDefaults';
import { usePresentationMode } from '../hooks/usePresentationMode';
import { api } from '../lib/api';
vi.mock('../lib/api',()=>({api:{getPresentationMode:vi.fn(async()=>({defaultMode:'auto',overrideMode:null,effectiveMode:'auto'})),setPresentationMode:vi.fn(async()=>{})}}));
function MountedChat(){const p=usePresentationMode('c');return <output>{p.effectiveMode}</output>;}
describe('global presentation preference',()=>{
 it('updates a still-mounted chat when settings saves and ignores a stale initial read',async()=>{
  let finish!:(v:unknown)=>void;
  vi.mocked(api.getPresentationMode).mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve as typeof finish;}));
  render(<><PresentationDefaults/><MountedChat/></>);
  fireEvent.change(screen.getByRole('combobox'),{target:{value:'visual'}});
  await waitFor(()=>expect(screen.getByRole('status')).toHaveTextContent('visual'));
  await act(async()=>finish({defaultMode:'auto',overrideMode:null,effectiveMode:'auto'}));
  expect(screen.getByRole('combobox')).toHaveValue('visual');
  expect(api.setPresentationMode).toHaveBeenCalledWith(null,'visual');
 });
 it('refreshes mounted chat when a save finishes after the Settings modal closes',async()=>{
  let finish!:()=>void;vi.mocked(api.setPresentationMode).mockImplementationOnce(()=>new Promise<void>(resolve=>{finish=resolve;}));
  const {rerender}=render(<><PresentationDefaults/><MountedChat/></>);
  fireEvent.change(screen.getByRole('combobox'),{target:{value:'text'}});
  await waitFor(()=>expect(finish).toBeTypeOf('function'));
  rerender(<>{false}<MountedChat/></>);
  await act(async()=>finish());
  await waitFor(()=>expect(screen.getByRole('status')).toHaveTextContent('text'));
 });

});
