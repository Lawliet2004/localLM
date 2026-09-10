// Minimal-mode gate (static): the Minimal preset exposes exactly the
// benchmark surface (execution source + edit_file + terminal_*) and nothing
// model-visible beyond it. Live model runs are a separate, credentialed gate.
import { readFileSync } from 'node:fs';

const presets = readFileSync('src-tauri/src/presets.rs', 'utf8');
const start = presets.indexOf('MINIMAL =>');
const end = presets.indexOf('CREATOR =>');
if (start < 0 || end < 0 || end < start) throw new Error('minimal preset block missing');
const minimal = presets.slice(start, end);
for (const must of ['sources: vec!["__execution"', 'mcp: false', 'system_time: false', 'skills: false', '"terminal_create"', '"terminal_send"', '"terminal_close"']) {
  if (!minimal.includes(must)) throw new Error(`minimal preset lost: ${must}`);
}
for (const banned of ['"subagent"', '"workflow_run"', '"memory_teach"', '"web_fetch"']) {
  if (minimal.includes(banned)) throw new Error(`minimal preset leaks: ${banned}`);
}
if (!presets.includes('["edit_file"]')) throw new Error('minimal workspace surface is not edit_file-only');
console.log('minimal-bench gate passed: Minimal = execution + edit_file + terminal_* only.');
