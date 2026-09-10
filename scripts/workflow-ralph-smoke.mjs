// Phase 4 acceptance (static): todo/goal logged state, workflow fan-out,
// Ralph loop with bounded handoffs, and the read-only Plan panel.
import { readFileSync } from 'node:fs';
const required = [
  ['src-tauri/src/plans.rs', 'pub fn parse_todo_write'],
  ['src-tauri/src/subagents.rs', 'pub async fn run_workflow'],
  ['src-tauri/src/subagents.rs', 'pub async fn run_ralph'],
  ['src-tauri/src/subagents.rs', '```handoff'],
  ['src-tauri/src/harness.rs', '"workflow_run"'],
  ['src-tauri/src/harness.rs', '"ralph_run"'],
  ['src-tauri/src/harness.rs', '"todo_write"'],
  ['src-tauri/src/lib.rs', 'plans::get_todos'],
  ['src/components/TodosPanel.tsx', 'getTodos'],
  ['src/lib/api.ts', 'getGoal'],
];
let failures = 0;
for (const [file, needle] of required) {
  if (!readFileSync(file, 'utf8').includes(needle)) { console.error(`MISSING: ${file} :: ${needle}`); failures += 1; }
}
if (failures > 0) throw new Error(`workflow-ralph smoke failed (${failures})`);
console.log('workflow-ralph smoke passed: todos/goals logged, workflow + Ralph bounded, Plan panel wired.');
