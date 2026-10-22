// node --import ./scripts/register-test-loader.mjs scripts/benchmark-markdown.mjs
import { performance } from 'node:perf_hooks';
import { fromMarkdown } from 'mdast-util-from-markdown';
import { MarkdownBlockBuffer } from '../src/lib/markdownBlocks.ts';
const prefix = Array.from({ length: 300 }, (_, index) => `Paragraph ${index}: **Completed prose** with a clear explanation and a short list of facts.\n\n`).join('');
const steps = Array.from({ length: 100 }, (_, index) => `${prefix}${'Current streamed words. '.repeat(index + 1)}`);
function median(run) { const samples = []; for (let i = 0; i < 3; i++) { const start = performance.now(); run(); samples.push(performance.now() - start); } return Number(samples.sort((a,b) => a-b)[1].toFixed(2)); }
const buffer = new MarkdownBlockBuffer(); for (const source of steps) buffer.update(source);
console.log(JSON.stringify({
  runtime: process.version, updates: steps.length, initialCharacters: prefix.length,
  fullDocumentBoundaryParseMedianMs: median(() => { for (const source of steps) fromMarkdown(source); }),
  incrementalBoundaryParseMedianMs: median(() => { const buffer = new MarkdownBlockBuffer(); for (const source of steps) buffer.update(source); }),
  previousBoundaryCharacters: steps.reduce((sum, source) => sum + source.length, 0),
  incrementalBoundaryCharacters: buffer.parsedCharacters,
  scope: 'CommonMark boundary parsing CPU, three samples; excludes React, GFM/math rendering, Tauri and inference. The production renderer remains react-markdown.',
}, null, 2));
