// Phase 3 acceptance (static): subagent spawn/fork paths, depth cap that
// errors loudly at the cap, background settle notices, lineage IPC, and the
// Trajectory team-tree. Live fan-out needs a loaded model; fixtures never
// stand in for it.
import { readFileSync } from 'node:fs';
const required = [
  ['src-tauri/src/subagents.rs', 'pub fn spawn_background'],
  ['src-tauri/src/subagents.rs', 'pub async fn run_child_inline'],
  ['src-tauri/src/harness.rs', 'Subagent depth cap reached'],
  ['src-tauri/src/subagents.rs', 'background child settled'],
  ['src-tauri/src/subagents.rs', 'pub fn interrupt_subagent'],
  ['src-tauri/src/harness.rs', '"send_message"'],
  ['src-tauri/src/harness.rs', 'exact direct parent'],
  ['src-tauri/src/lib.rs', 'subagents::list_subagent_runs'],
  ['src/components/Trajectory.tsx', 'agent-tree'],
  ['src/lib/api.ts', 'listSubagentRuns'],
];
let failures = 0;
for (const [file, needle] of required) {
  if (!readFileSync(file, 'utf8').includes(needle)) { console.error(`MISSING: ${file} :: ${needle}`); failures += 1; }
}
if (failures > 0) throw new Error(`subagent-parallel smoke failed (${failures})`);
console.log('subagent-parallel smoke passed: spawn/fork, depth cap, settle notices, lineage, team-tree.');
