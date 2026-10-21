import { readFile } from 'node:fs/promises';
import ts from 'typescript';

export async function load(url, context, nextLoad) {
  if (new URL(url).pathname.endsWith('.ts')) {
    const source = await readFile(new URL(url), 'utf8');
    return {
      format: 'module', shortCircuit: true,
      source: ts.transpileModule(source, {
        compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
      }).outputText,
    };
  }
  return nextLoad(url, context);
}
