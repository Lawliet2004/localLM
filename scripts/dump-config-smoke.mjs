// Static acceptance for the Phase 0 capability seam + Trajectory viewer.
// Runs without the native app: asserts the IPC seam is registered, the
// system_time kill-switch is enforced in the agent loop, and the read-only
// viewer exists. Fail loud; no fixtures claim runtime behavior.
import { readFileSync } from 'node:fs';

const required = [
  ['src-tauri/src/lib.rs', 'capabilities::list_capabilities'],
  ['src-tauri/src/lib.rs', 'capabilities::set_capability_enabled'],
  ['src-tauri/src/lib.rs', 'capabilities::dump_config'],
  ['src-tauri/src/capabilities.rs', 'is_enabled(&store, "system_time")'],
  ['src-tauri/src/chat.rs', 'capabilities::is_enabled'],
  ['src/components/Trajectory.tsx', 'getRunEvents'],
  ['src/lib/api.ts', 'dumpConfig'],
];
let failures = 0;
for (const [file, needle] of required) {
  const content = readFileSync(file, 'utf8');
  if (!content.includes(needle)) {
    console.error(`MISSING: ${file} does not contain ${needle}`);
    failures += 1;
  }
}
if (failures > 0) throw new Error(`dump-config smoke failed with ${failures} missing seam(s)`);
console.log('dump-config smoke passed: capability IPC registered, system_time gated, Trajectory viewer wired.');
