// Phase 7 acceptance (static): cron parser, schedules CRUD + unattended
// execution, loopback webhook/API, headless CLI queue + wait.
import { readFileSync } from 'node:fs';
const required = [
  ['src-tauri/src/scheduling.rs', 'pub fn cron_next'],
  ['src-tauri/src/scheduling.rs', 'pub async fn execute_schedule'],
  ['src-tauri/src/scheduling.rs', '/api/schedules/run'],
  ['src-tauri/src/scheduling.rs', 'Bearer'],
  ['src-tauri/src/scheduling.rs', 'pub async fn run_background'],
  ['src-tauri/src/main.rs', '--profile headless'],
  ['src-tauri/src/main.rs', '--wait'],
  ['src-tauri/src/lib.rs', 'scheduling::run_schedule_now'],
  ['src/components/Schedules.tsx', 'runScheduleNow'],
  ['python/locallm_sdk.py', 'wait_for_result'],
];
let failures = 0;
for (const [file, needle] of required) {
  if (!readFileSync(file, 'utf8').includes(needle)) { console.error(`MISSING: ${file} :: ${needle}`); failures += 1; }
}
if (failures > 0) throw new Error(`scheduling-webhook smoke failed (${failures})`);
console.log('scheduling-webhook smoke passed: cron, unattended runs, loopback API, headless CLI, Python wait.');
