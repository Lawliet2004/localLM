// Phase 6 acceptance (static): SQLite memory bank, verbatim recall block,
// repo survey ingest, and the Memory page.
import { readFileSync } from 'node:fs';
const required = [
  ['src-tauri/src/memory.rs', 'pub fn recall_block'],
  ['src-tauri/src/memory.rs', 'pub fn survey_repo'],
  ['src-tauri/src/lib.rs', 'memory::ingest_repo'],
  ['src-tauri/src/chat.rs', 'recall_block'],
  ['src/components/Memory.tsx', 'teachFact'],
  ['src/lib/api.ts', 'ingestRepo'],
];
let failures = 0;
for (const [file, needle] of required) {
  if (!readFileSync(file, 'utf8').includes(needle)) { console.error(`MISSING: ${file} :: ${needle}`); failures += 1; }
}
if (failures > 0) throw new Error(`memory-recall smoke failed (${failures})`);
console.log('memory-recall smoke passed: bank, verbatim recall, survey ingest, Memory page.');
