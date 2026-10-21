import { readFileSync } from 'node:fs';
import { render,cleanup } from '@testing-library/react';
import { describe,it,expect } from 'vitest';
import { repairProgram } from './repair';
import { validateProgram } from './validation';
import { ValidatedRenderer } from './ValidatedRenderer';
const measured=JSON.parse(readFileSync('docs/benchmarks/openui-gemma-cpu.json','utf8')) as {cases:{name:string;profile:string|null;source:string}[]};
describe('actual configured GGUF output fixtures',()=>{
 for(const sample of measured.cases.filter(c=>c.profile))it(`validates and renders measured ${sample.name} output`,()=>{
  const prepared=repairProgram(sample.source);expect(validateProgram(prepared.program).valid).toBe(true);
  const result=render(<ValidatedRenderer response={prepared.program}/>);expect(result.container.textContent).not.toContain('Couldn’t');expect(result.container.textContent?.length).toBeGreaterThan(0);cleanup();
 });
});
