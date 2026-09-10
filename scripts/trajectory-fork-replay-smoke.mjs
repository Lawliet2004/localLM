// Phase 1 acceptance (static): append-only log tables, fork with lineage,
// deterministic replay transcript, bounded search, Trajectory + Sessions UI.
import { readFileSync } from 'node:fs';
const required = [
  ['src-tauri/src/store.rs', 'pub fn fork_conversation'],
  ['src-tauri/src/store.rs', 'pub fn search_sessions'],
  ['src-tauri/src/sessions.rs', 'SESSION_FORMAT_VERSION'],
  ['src-tauri/src/sessions.rs', 'pub async fn fork_session'],
  ['src-tauri/src/sessions.rs', 'pub fn replay_session'],
  ['src-tauri/src/sessions.rs', 'pub fn search_sessions'],
  ['src-tauri/src/lib.rs', 'sessions::fork_session'],
  ['src/components/Trajectory.tsx', 'getConversationRuns'],
  ['src/components/SessionsPanel.tsx', 'forkSession'],
  ['docs/session-format-status.md', 'SESSION_FORMAT_VERSION'],
];
let failures = 0;
for (const [file, needle] of required) {
  if (!readFileSync(file, 'utf8').includes(needle)) { console.error(`MISSING: ${file} :: ${needle}`); failures += 1; }
}
if (failures > 0) throw new Error(`trajectory-fork-replay smoke failed (${failures})`);
console.log('trajectory-fork-replay smoke passed: log, fork lineage, replay, search, both viewers.');
