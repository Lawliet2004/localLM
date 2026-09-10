// Phase 5 acceptance (static): Docker boundary, persistent terminal, FS
// deny-list hook, bounded web tools, loop-hygiene + timeout guards.
import { readFileSync } from 'node:fs';
const required = [
  ['src-tauri/src/sandbox.rs', 'pub async fn docker_exec'],
  ['src-tauri/src/sandbox.rs', '--network", "none'],
  ['src-tauri/src/sandbox.rs', 'pub async fn send'],
  ['src-tauri/src/workspace.rs', 'check_fs_path'],
  ['src-tauri/src/sandbox.rs', 'pub async fn web_fetch'],
  ['src-tauri/src/sandbox.rs', 'pub fn file_search'],
  ['src-tauri/src/sandbox.rs', 'pub fn check_repetition'],
  ['src-tauri/src/sandbox.rs', 'pub fn effective_timeout_secs'],
  ['src-tauri/src/chat.rs', 'check_repetition'],
  ['src/components/SandboxPanel.tsx', 'setSandboxProvider'],
];
let failures = 0;
for (const [file, needle] of required) {
  if (!readFileSync(file, 'utf8').includes(needle)) { console.error(`MISSING: ${file} :: ${needle}`); failures += 1; }
}
if (failures > 0) throw new Error(`sandbox-terminal smoke failed (${failures})`);
console.log('sandbox-terminal smoke passed: Docker, terminal, FS policy, web tools, guards.');
