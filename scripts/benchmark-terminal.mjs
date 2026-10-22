// Run: node --import ./scripts/register-test-loader.mjs scripts/benchmark-terminal.mjs
// Optional baseline: LOCALLM_ANSI_BASELINE=/absolute/path/to/old-ansi.ts
import { performance } from 'node:perf_hooks';
import { pathToFileURL } from 'node:url';
import { AnsiStreamParser, AccumulatedAnsiParser } from '../src/lib/ansi.ts';
const chunk = '\u001b[32mPASS\u001b[0m fixture: output streamed safely\n'.repeat(12);
const chunks = Array.from({ length: 500 }, () => chunk);
const baseline = process.env.LOCALLM_ANSI_BASELINE ? await import(pathToFileURL(process.env.LOCALLM_ANSI_BASELINE)) : null;
function measure(run) {
  const samples = [];
  for (let i = 0; i < 3; i++) { const start = performance.now(); run(); samples.push(performance.now() - start); }
  return Number(samples.sort((a,b) => a-b)[1].toFixed(2));
}
const parser = new AnsiStreamParser();
for (const part of chunks) parser.append(part);
const result = {
  runtime: process.version, chunks: chunks.length, bytes: Buffer.byteLength(chunks.join('')),
  processedCharacters: parser.processedChars,
  incrementalMedianMs: measure(() => { const parser = new AnsiStreamParser(); for (const part of chunks) parser.append(part); }),
  accumulatedAdapterMedianMs: measure(() => { const parser = new AccumulatedAnsiParser(); let output = ''; for (const part of chunks) { output += part; parser.update(output); } }),
  ...(baseline ? { previousFullParseMedianMs: measure(() => { let output = ''; for (const part of chunks) { output += part; baseline.parseAnsi(output); } }) } : {}),
  scope: 'Synthetic ANSI parsing CPU only, 3 samples. Excludes React, layout, Tauri, inference and tool execution.',
};
console.log(JSON.stringify(result, null, 2));
