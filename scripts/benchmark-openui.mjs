// Offline CPU/bundle measurements only. No server, browser or model is started.
import { readFileSync, readdirSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { performance } from 'node:perf_hooks';
import { cpus, totalmem } from 'node:os';
import { runnerImport } from 'vite';

const { module: { validateProgram } } = await runnerImport('./src/genui/validation.ts', { configFile: false });
const { module: { progressiveProgram } } = await runnerImport("./src/genui/progressive.ts", { configFile: false });
const cases = {
  core: 'root = Response([Heading("TCP"), Text("A reliable transport protocol.")])',
  data: 'root = Response([Table(["Month", "Sales"], [["Jan",10],["Feb",15],["Mar",25]])])',
  coding: 'root = Response([CodeBlock("def identity(value):\\n    return value", "python")])',
  invalid: 'root = Response([Progress("Usage", 999)])',
};
const percentile = (values, fraction) => values.sort((a, b) => a - b)[Math.floor((values.length - 1) * fraction)];
const results = {};
for (const [name, source] of Object.entries(cases)) {
  for (let i = 0; i < 30; i++) validateProgram(source);
  const times = [];
  for (let i = 0; i < 300; i++) {
    const start = performance.now();
    const validation = validateProgram(source);
    times.push(performance.now() - start);
    if (validation.valid !== (name !== 'invalid')) throw new Error(`Unexpected ${name} validation result`);
  }
  results[name] = { sourceChars: source.length, validationP50Ms: percentile(times, 0.5), validationP95Ms: percentile(times, 0.95) };
}
const partial='root = Response(['+Array.from({length:24},(_,i)=>`Text("Section ${i}")`).join(',')+', Chart("Sales","bar",[';
const streamingTimes=[];
for(let i=0;i<100;i++){const start=performance.now();if(!progressiveProgram(partial))throw new Error('Missing committed streaming prefix');streamingTimes.push(performance.now()-start);}
results.streamingPrefix={sourceChars:partial.length,validationP50Ms:percentile(streamingTimes,.5),validationP95Ms:percentile(streamingTimes,.95),samples:100};
const prompts = Object.fromEntries(['full', 'core', 'data', 'education', 'coding'].map(profile => {
  const content = readFileSync(`src/genui/generated/${profile}.txt`, 'utf8');
  return [profile, { chars: content.length, bytes: Buffer.byteLength(content) }];
}));
const chunks = readdirSync('dist/assets').filter(file => /^(library|actions|ValidatedRenderer|VisualAnswer|GenUiBlock|GenUiSurface)-.*\.js$/.test(file)).map(file => {
  const content = readFileSync(`dist/assets/${file}`);
  return { file, bytes: content.length, gzipBytes: gzipSync(content).length };
});
console.log(JSON.stringify({ node: process.version,
  host: { platform: process.platform, arch: process.arch, cpu: cpus()[0]?.model, logicalCpus: cpus().length, totalMemoryBytes: totalmem() },
  samples: 300, results, prompts, chunks,
  nodeProcessPeakRssKiB: process.resourceUsage().maxRSS,
  limitations: 'Warm Node CPU validation; excludes React/browser paint, native app RAM, tokenizer counts and inference. Chunks are current build size, not a measured incremental baseline.' }, null, 2));
