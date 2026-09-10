// Phase 8 acceptance (static): Anthropic Messages adapter with tool_use
// translation, OpenAI-compatible hosts, explicit limits, compaction.
import { readFileSync } from 'node:fs';
const required = [
  ['src-tauri/src/inference.rs', 'pub fn anthropic_payload'],
  ['src-tauri/src/inference.rs', 'pub fn translate_anthropic_event'],
  ['src-tauri/src/inference.rs', 'input_json_delta'],
  ['src-tauri/src/inference.rs', 'Anthropic {'],
  ['src-tauri/src/providers.rs', 'ANTHROPIC_MESSAGES'],
  ['src-tauri/src/providers.rs', 'provider_formats'],
  ['src-tauri/src/chat.rs', 'ANTHROPIC_MESSAGES'],
  ['src-tauri/src/compaction.rs', 'pub fn compact_now'],
  ['src/components/ProviderManager.tsx', 'anthropic-messages'],
];
let failures = 0;
for (const [file, needle] of required) {
  if (!readFileSync(file, 'utf8').includes(needle)) { console.error(`MISSING: ${file} :: ${needle}`); failures += 1; }
}
if (failures > 0) throw new Error(`adapters smoke failed (${failures})`);
console.log('adapters smoke passed: Anthropic adapter + tool_use translation, formats, compaction.');
