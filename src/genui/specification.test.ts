import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { generateSystemPrompt } from '@openuidev/lang-core';
import { locallmLibrary } from './library';
import * as specification from './specification';

const profiles = ['full', 'core', 'data', 'coding', 'education', 'chart', 'code'] as const;

describe('generated library artifacts', () => {
  it.each(profiles)('%s prompt and official spec match the actual library', profile => {
    const library = specification[`${profile}Library`];
    const options = specification[`${profile}PromptOptions`];
    const spec = JSON.parse(readFileSync(`src/genui/generated/${profile}.spec.json`, 'utf8'));
    const prompt = readFileSync(`src/genui/generated/${profile}.txt`, 'utf8').trim();
    expect(spec).toEqual({ ...library.toSpec(), schema: library.toJSONSchema() });
    expect(prompt).toBe(library.prompt(options).trim());
    expect(prompt).toBe(generateSystemPrompt({ library: spec, promptOptions: options }).trim());
  });
  it('keeps React renderer schemas equal to the React-free CLI entry', () => {
    expect(locallmLibrary.toJSONSchema()).toEqual(specification.fullLibrary.toJSONSchema());
    expect(locallmLibrary.toSpec()).toEqual(specification.fullLibrary.toSpec());
  });
  it('ships exactly the generated full prompt to Rust', () => {
    expect(readFileSync('src-tauri/src/genui_prompt.txt', 'utf8'))
      .toBe(readFileSync('src/genui/generated/full.txt', 'utf8'));
  });
  it('prunes unavailable components from profile parent references', () => {
    const coreSchema = JSON.stringify(specification.coreLibrary.toJSONSchema());
    expect(coreSchema).not.toContain('#/$defs/CodeBlock');
    expect(coreSchema).not.toContain('#/$defs/Button');
    expect(specification.coreLibrary.prompt(specification.corePromptOptions).length)
      .toBeLessThan(specification.fullLibrary.prompt(specification.fullPromptOptions).length);
  });
});
