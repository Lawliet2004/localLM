// Phase 2 PTC acceptance (static): generated TS SDK, per-step policy checks,
// SDK/step consistency gate, TS + Python client surfaces.
import { readFileSync } from 'node:fs';
const required = [
  ['src-tauri/src/presets.rs', 'pub fn ts_sdk'],
  ['src-tauri/src/harness.rs', '"ptc_run"'],
  ['src-tauri/src/harness.rs', 'without a matching step'],
  ['src-tauri/src/harness.rs', 'cannot nest ptc_run'],
  ['src-tauri/src/chat.rs', 'ptc_sdk'],
  ['src/lib/sdk.ts', 'loopbackSdk'],
  ['python/locallm_sdk.py', 'run_schedule'],
];
let failures = 0;
for (const [file, needle] of required) {
  if (!readFileSync(file, 'utf8').includes(needle)) { console.error(`MISSING: ${file} :: ${needle}`); failures += 1; }
}
if (failures > 0) throw new Error(`ptc-sdk smoke failed (${failures})`);
console.log('ptc-sdk smoke passed: SDK generation, per-step approval, consistency gate, TS + Python clients.');
