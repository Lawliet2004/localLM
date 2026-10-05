// Explicit real-hardware acceptance; isolated databases and authenticated managed servers.
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fs from 'node:fs';
import path from 'node:path';

const mode = process.argv[2];
if (!['workload', 'soak', 'integration', 'max-context', 'streaming'].includes(mode)) throw new Error('Choose workload, soak, integration, max-context, or streaming.');
const root = path.resolve('.local/experimental/bonsai-27b');
const bundle = fs.readdirSync(root).find(n => n.startsWith('prism-b10754-2459f68-cuda12.4-'));
if (!bundle) throw new Error('Acquire the isolated pinned runtime first.');
const executable = path.join(root, bundle, 'llama-server.exe');
const quoted = executable.replaceAll("'", "''");
const evidence = path.resolve('.local/bonsai-evidence');
const samples = [];
const child = spawn('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--lib',
  'experimental::bonsai::qualification::laptop_qualification', '--', '--ignored', '--nocapture', '--test-threads=1'],
{ windowsHide: true, env: { ...process.env, BONSAI_QA_MODE: mode }, stdio: ['ignore', 'pipe', 'pipe'] });
const sampler = spawn('powershell.exe', ['-NoProfile', '-Command',
  `$ErrorActionPreference='Stop'; while ($true) { Get-Process -Name llama-server -ErrorAction SilentlyContinue | ForEach-Object { if ($_.Path -eq '${quoted}') { Write-Output "$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()),$($_.Id),$($_.WorkingSet64),$($_.PrivateMemorySize64)" } }; Start-Sleep -Milliseconds 500 }`],
{ windowsHide: true, stdio: ['ignore', 'pipe', 'ignore'] });
let pending = '';
sampler.stdout.on('data', bytes => {
  pending += bytes;
  const lines = pending.split(/\r?\n/); pending = lines.pop();
  for (const line of lines) { const [time, pid, workingSetBytes, privateBytes] = line.split(',').map(Number);
    if ([time, pid, workingSetBytes, privateBytes].every(Number.isFinite)) samples.push({ time, pid, workingSetBytes, privateBytes }); }
});
let busy = false;
const timer = setInterval(() => {
  if (busy) return; busy = true;
  const query = spawn('nvidia-smi', ['--query-gpu=memory.used', '--format=csv,noheader,nounits', '-i', '0'], { windowsHide: true });
  let data = ''; query.stdout.on('data', bytes => { data += bytes; });
  query.on('close', () => { const gpuMib = Number(data.trim()); if (data.trim() && Number.isFinite(gpuMib)) samples.push({ time: Date.now(), gpuMib }); busy = false; });
  query.on('error', () => { busy = false; });
}, 500);
let stdout = '', stderr = '', killed = false;
child.stdout.on('data', bytes => {
  stdout += bytes; process.stdout.write(bytes);
  if (!killed && stdout.includes('BONSAI_QA_KILL_READY')) {
    killed = true;
    // The human requested a mid-generation kill. Match the exact experimental executable first.
    const kill = spawn('powershell.exe', ['-NoProfile', '-Command',
      `$bonsaiTargets=@(Get-Process -Name llama-server -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq '${quoted}' }); if ($bonsaiTargets.Count -ne 1) { throw 'Expected exactly one isolated Bonsai process' }; $bonsaiTargets | ForEach-Object { Write-Output "Killed isolated Bonsai PID $($_.Id)"; Stop-Process -Id $_.Id -Force }`], { windowsHide: true });
    kill.stdout.on('data', b => process.stdout.write(b)); kill.stderr.on('data', b => process.stderr.write(b));
  }
});
child.stderr.on('data', bytes => { stderr += bytes; process.stderr.write(bytes); });
const checkpoint = setInterval(() => fs.writeFileSync(path.join(evidence, `qa-${mode}-memory.json`), JSON.stringify(samples)), 10000);
const [exitCode, signal] = await once(child, 'close');
clearInterval(timer); clearInterval(checkpoint); sampler.kill();
fs.writeFileSync(path.join(evidence, `qa-${mode}.stdout.txt`), stdout);
fs.writeFileSync(path.join(evidence, `qa-${mode}.stderr.txt`), stderr);
fs.writeFileSync(path.join(evidence, `qa-${mode}-memory.json`), JSON.stringify(samples));
const peak = field => Math.max(0, ...samples.map(s => s[field] ?? 0));
const report = { mode, exitCode, signal, peakGpuMib: peak('gpuMib'), peakWorkingSetBytes: peak('workingSetBytes'), peakPrivateBytes: peak('privateBytes'), sampleCount: samples.length };
fs.writeFileSync(path.join(evidence, `qa-${mode}-result.json`), JSON.stringify(report, null, 2));
console.log(JSON.stringify(report));
process.exitCode = exitCode ?? 1;
