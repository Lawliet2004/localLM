// Isolated hardware acceptance. Flags come from the acquired fork's --help.
import { spawn, execFileSync } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import path from 'node:path';

const root = path.resolve('.local/experimental/bonsai-27b');
const evidence = path.resolve('.local/bonsai-evidence');
const runtime = fs.readdirSync(root).find(name => name.startsWith('prism-b10754-2459f68-cuda12.4-'));
if (!runtime) throw new Error('Acquire the pinned Bonsai runtime first.');
const executable = path.join(root, runtime, 'llama-bench.exe');
const model = path.join(root, 'models/Bonsai-27B-Q1_0.gguf');
if (!fs.existsSync(model) || fs.statSync(model).size !== 3803452480) {
  throw new Error('The full verified model must be published before benchmarking.');
}
fs.mkdirSync(evidence, { recursive: true });
const devices = execFileSync(executable, ['--list-devices'], { encoding: 'utf8', windowsHide: true });
if (!devices.includes('CUDA0: NVIDIA GeForce RTX 2050')) throw new Error(`Expected dGPU missing: ${devices}`);
const base = { layers: 32, context: 2048, k: 'f16', v: 'f16', kvCpu: false, flash: 'on', batch: 128, ubatch: 64, threads: 6, load: 'mmap' };

async function benchmark(name, config, prompt, generation, repetitions) {
  // Run pp and tg separately so each allocates the exact requested context.
  const results = [];
  for (const [kind, pp, tg] of [['pp', prompt, 0], ['tg', 0, generation]]) {
    const flags = ['-m', model, '-dev', 'CUDA0', '-ngl', String(config.layers),
      '-p', String(pp), '-n', String(tg), '-d', String(Math.max(0, config.context - pp - tg)),
      '-r', String(repetitions), '-o', 'json', '-b', String(config.batch), '-ub', String(config.ubatch),
      '-t', String(config.threads), '-ctk', config.k, '-ctv', config.v, '-fa', config.flash,
      '-nkvo', config.kvCpu ? '1' : '0', '-lm', config.load];
    console.log(`Starting ${name}/${kind}: ${JSON.stringify(config)}`);
    const child = spawn(executable, flags, { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '', stderr = '';
    let peakGpuMib = null, peakWorkingSetBytes = null, peakPrivateBytes = null;
    let gpuSamples = 0, ramSamples = 0;
    child.stdout.on('data', data => { stdout += data; });
    child.stderr.on('data', data => { stderr += data; });
    // A separate hidden sampler records process memory. The sampler owns no model.
    const sampler = spawn('powershell.exe', ['-NoProfile', '-Command',
      `$ErrorActionPreference='Stop'; while ($true) { $bonsaiProcess=Get-Process -Id ${child.pid} -ErrorAction SilentlyContinue; if (-not $bonsaiProcess) { break }; Write-Output "$($bonsaiProcess.WorkingSet64),$($bonsaiProcess.PrivateMemorySize64)"; Start-Sleep -Milliseconds 500 }`],
    { windowsHide: true, stdio: ['ignore', 'pipe', 'ignore'] });
    let pending = '';
    sampler.stdout.on('data', data => {
      pending += data;
      const lines = pending.split(/\r?\n/); pending = lines.pop();
      for (const line of lines) {
        const [working, privateBytes] = line.split(',').map(Number);
        if (Number.isFinite(working) && Number.isFinite(privateBytes)) {
          peakWorkingSetBytes = Math.max(peakWorkingSetBytes ?? 0, working);
          peakPrivateBytes = Math.max(peakPrivateBytes ?? 0, privateBytes); ramSamples++;
        }
      }
    });
    let gpuBusy = false;
    const gpuTimer = setInterval(() => {
      if (gpuBusy) return;
      gpuBusy = true;
      const query = spawn('nvidia-smi', ['--query-gpu=memory.used', '--format=csv,noheader,nounits', '-i', '0'],
        { windowsHide: true, stdio: ['ignore', 'pipe', 'ignore'] });
      let data = '';
      query.stdout.on('data', chunk => { data += chunk; });
      query.on('close', () => { const n = Number(data.trim()); if (data.trim() && Number.isFinite(n)) { peakGpuMib = Math.max(peakGpuMib ?? 0, n); gpuSamples++; } gpuBusy = false; });
      query.on('error', () => { gpuBusy = false; });
    }, 500);
    const deadline = setTimeout(() => child.kill(), 20 * 60 * 1000);
    const [exitCode, signal] = await once(child, 'close');
    clearTimeout(deadline); clearInterval(gpuTimer); sampler.kill();
    fs.writeFileSync(path.join(evidence, `bench-${name}-${kind}.stdout.json`), stdout);
    fs.writeFileSync(path.join(evidence, `bench-${name}-${kind}.stderr.txt`), stderr);
    let measurements = null;
    try { measurements = JSON.parse(stdout); } catch { /* Retain actual failure output. */ }
    const record = { name, kind, config, flags, exitCode, signal, measurements,
      peakGpuMib, peakWorkingSetBytes, peakPrivateBytes, gpuSamples, ramSamples };
    results.push(record);
    fs.writeFileSync(path.join(evidence, `bench-${name}-${kind}.result.json`), JSON.stringify(record, null, 2));
    console.log(JSON.stringify({ name, kind, exitCode, peakGpuMib, peakWorkingSetBytes, peakPrivateBytes,
      tokensPerSecond: measurements?.map(m => m.avg_ts), error: exitCode ? stderr.slice(-1600) : undefined }));
    if (exitCode !== 0) break;
  }
  return results;
}

const mode = process.argv[2] ?? 'probe';
if (mode === 'probe') {
  const result = await benchmark('probe', { ...base, context: 128 }, 16, 8, 1);
  if (result.some(r => r.exitCode !== 0)) process.exitCode = 1;
} else if (mode === 'layers') {
  for (const layers of [65, 56, 48, 40, 32, 16, 0]) {
    await benchmark(`layers-${layers}`, { ...base, layers }, 128, 64, 1);
  }
} else if (mode === 'setting') {
  const config = { ...base, ...JSON.parse(process.argv[3] ?? '{}') };
  await benchmark(process.argv[4] ?? 'setting', config, 512, 128, 3);
} else {
  throw new Error('Choose probe, layers, or setting with a JSON configuration.');
}
