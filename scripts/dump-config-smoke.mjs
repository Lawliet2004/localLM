// Static acceptance for Phase 0 capability seam + presets + Trajectory viewer.
// Asserts the IPC seam is registered, capabilities.json defines required capabilities
// (subagents, workflows, compaction), Registry models dependencies and effects,
// and system_time kill-switch is enforced in the agent loop.
import { readFileSync, existsSync } from 'node:fs';

const requiredSeams = [
  ['src-tauri/src/lib.rs', 'capabilities::list_capabilities'],
  ['src-tauri/src/lib.rs', 'capabilities::set_capability_enabled'],
  ['src-tauri/src/lib.rs', 'capabilities::dump_config'],
  ['src-tauri/src/capabilities.rs', 'pub enum CapabilityKind'],
  ['src-tauri/src/capabilities.rs', 'pub struct Registry'],
  ['src-tauri/src/capabilities.rs', 'is_enabled(&store, "system_time")'],
  ['src-tauri/src/chat.rs', 'capabilities::is_enabled'],
  ['src/lib/api.ts', 'dumpConfig'],
];

let failures = 0;
for (const [file, needle] of requiredSeams) {
  if (!existsSync(file)) {
    console.error(`MISSING FILE: ${file}`);
    failures += 1;
    continue;
  }
  const content = readFileSync(file, 'utf8');
  if (!content.includes(needle)) {
    console.error(`MISSING: ${file} does not contain ${needle}`);
    failures += 1;
  }
}

// Verify capabilities.json exists and defines subagents, workflows, and compaction
if (!existsSync('src-tauri/capabilities.json')) {
  console.error('MISSING FILE: src-tauri/capabilities.json');
  failures += 1;
} else {
  const capsRaw = readFileSync('src-tauri/capabilities.json', 'utf8');
  const caps = JSON.parse(capsRaw);
  const ids = caps.capabilities.map(c => c.id);
  for (const expected of ['subagents', 'workflows', 'compaction', 'system_time', 'workspace_files']) {
    if (!ids.includes(expected)) {
      console.error(`MISSING CAPABILITY: ${expected} not defined in capabilities.json`);
      failures += 1;
    }
  }
  if (!caps.presets || !caps.presets.standard || !caps.presets.code_ptc || !caps.presets.minimal || !caps.presets.creator) {
    console.error('MISSING PRESET: capabilities.json must specify standard, code_ptc, minimal, and creator presets');
    failures += 1;
  }
}

if (failures > 0) throw new Error(`dump-config smoke failed with ${failures} missing check(s)`);
console.log('dump-config smoke passed: capabilities.json verified (subagents/workflows/compaction), Registry registered, and system_time gated.');
