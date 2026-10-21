// Build-time only. The official CLI emits prompt + spec from one catalog.
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);
const cli = join(dirname(require.resolve('@openuidev/cli/package.json')), 'dist', 'index.js');
const generated = join(root, 'src', 'genui', 'generated');
mkdirSync(generated, { recursive: true });
for (const profile of ['full', 'core', 'data', 'coding', 'education', 'chart', 'code']) {
  const output = join(generated, `${profile}.txt`);
  execFileSync(process.execPath, [cli, 'generate', './src/genui/specification.ts',
    '--export', `${profile}Library`, '--prompt-options', `${profile}PromptOptions`,
    '--no-interactive', '--out', output], {
    cwd: root,
    env: { ...process.env, OPENUI_TELEMETRY_DISABLED: '1', DO_NOT_TRACK: '1' },
    stdio: 'inherit',
  });
}
// Rust includes static text; no runtime Node process or prompt construction.
writeFileSync(join(root, 'src-tauri', 'src', 'genui_prompt.txt'),
  readFileSync(join(generated, 'full.txt')));
