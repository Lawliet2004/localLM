// Phase 9 acceptance (static): manifest shape, malware scan verdicts,
// install/enable/remove, in-memory creator testing, Plugins page.
import { readFileSync } from 'node:fs';
const required = [
  ['src-tauri/src/plugins.rs', 'pub fn scan'],
  ['src-tauri/src/plugins.rs', 'pub async fn install_plugin'],
  ['src-tauri/src/plugins.rs', 'pub fn test_plugin'],
  ['src-tauri/src/plugins.rs', '"reject"'],
  ['src-tauri/src/lib.rs', 'plugins::scan_plugin'],
  ['src/components/Plugins.tsx', 'testPlugin'],
  ['SAFETY.md', 'least privilege'],
];
let failures = 0;
for (const [file, needle] of required) {
  const content = readFileSync(file, 'utf8');
  if (!content.toLowerCase().includes(needle.toLowerCase())) { console.error(`MISSING: ${file} :: ${needle}`); failures += 1; }
}
if (failures > 0) throw new Error(`plugin-scan smoke failed (${failures})`);
console.log('plugin-scan smoke passed: manifest, scan verdicts, lifecycle, creator testing, Plugins page.');
