// Smoke test verifying Phase 1: Append-only session event log, trajectory viewer,
// session forking, and session event replay.
import { readFileSync, existsSync } from 'node:fs';

const requiredSeams = [
  // Backend IPC registration in lib.rs
  ['src-tauri/src/lib.rs', 'sessions::get_session_events'],
  ['src-tauri/src/lib.rs', 'sessions::fork_session'],
  ['src-tauri/src/lib.rs', 'sessions::replay_session'],
  ['src-tauri/src/lib.rs', 'sessions::search_sessions'],

  // Backend persistence in store.rs
  ['src-tauri/src/store.rs', 'CREATE TABLE IF NOT EXISTS session_events'],
  ['src-tauri/src/store.rs', 'CREATE INDEX IF NOT EXISTS session_events_conv_seq'],
  ['src-tauri/src/store.rs', 'fn migrate_historical_to_session_events'],
  ['src-tauri/src/store.rs', 'pub fn next_session_event_seq'],
  ['src-tauri/src/store.rs', 'pub fn append_session_event'],
  ['src-tauri/src/store.rs', 'pub fn session_events'],
  ['src-tauri/src/store.rs', 'pub fn fork_session_events'],
  ['src-tauri/src/store.rs', 'pub fn search_sessions'],

  // Chat emission in chat.rs
  ['src-tauri/src/chat.rs', 'fn emit_session_event'],
  ['src-tauri/src/chat.rs', '"user_msg"'],
  ['src-tauri/src/chat.rs', '"system_prompt"'],
  ['src-tauri/src/chat.rs', '"context_injection"'],
  ['src-tauri/src/chat.rs', '"turn_start"'],
  ['src-tauri/src/chat.rs', '"step_start"'],
  ['src-tauri/src/chat.rs', '"reasoning"'],
  ['src-tauri/src/chat.rs', '"tool_call"'],
  ['src-tauri/src/chat.rs', '"tool_result"'],
  ['src-tauri/src/chat.rs', '"step_end"'],
  ['src-tauri/src/chat.rs', '"turn_end"'],

  // Frontend API in api.ts
  ['src/lib/api.ts', 'getSessionEvents'],
  ['src/lib/api.ts', 'forkSession'],
  ['src/lib/api.ts', 'replaySession'],
  ['src/lib/api.ts', 'searchSessions'],

  // Frontend types in types.ts
  ['src/lib/types.ts', 'export interface SessionEvent'],
  ['src/lib/types.ts', 'export interface ForkSessionResult'],
  ['src/lib/types.ts', 'export interface Hit'],

  // Trajectory component in Trajectory.tsx
  ['src/components/Trajectory.tsx', 'export function Trajectory'],
  ['src/components/Trajectory.tsx', 'api.getSessionEvents'],
  ['src/components/Trajectory.tsx', 'api.forkSession'],
  ['src/components/Trajectory.tsx', "'all', 'model', 'tool', 'system', 'error'"],
  ['src/components/Trajectory.tsx', 'Fork from here'],

  // App integration in App.tsx
  ['src/App.tsx', '<Trajectory'],
  ['src/App.tsx', 'onFork='],

  // Trajectory styles in styles.css
  ['src/styles.css', '.trajectory'],
  ['src/styles.css', '.trajectory-toolbar'],
  ['src/styles.css', '.trajectory-fork-btn'],

  // Trajectory test in Trajectory.test.tsx
  ['src/components/Trajectory.test.tsx', 'getSessionEvents'],
  ['src/components/Trajectory.test.tsx', 'forkSession'],
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
    console.error(`MISSING: ${file} does not contain "${needle}"`);
    failures += 1;
  }
}

if (failures > 0) {
  throw new Error(`trajectory-fork-replay smoke failed with ${failures} missing check(s)`);
}

console.log('trajectory-fork-replay smoke passed: all Phase 1 session event, trajectory viewer, forking, and replay seams verified.');
