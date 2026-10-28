// Run bundled Tauri/WebKit on an owned X11/D-Bus display; no browser preview.
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFile, mkdir, writeFile, open } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline';
const output = resolve(process.env.LOCALLM_NATIVE_OUTPUT || '.local/qa/coding-agent-native');
const binary = resolve(process.env.LOCALLM_NATIVE_BINARY || 'src-tauri/target/debug/locallm');
await mkdir(output, { recursive: true });
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const provenance = { binary, binarySha256: sha256(await readFile(binary)), frontendSha256: sha256(await readFile('dist/index.html')), startedAt: new Date().toISOString(), renderer: 'bundled native Tauri/WebKit', fixture: 'authored persisted workflow; backend execution is verified separately', browserUsed: false };
await writeFile(resolve(output, 'provenance.json'), JSON.stringify(provenance, null, 2));
const display = spawn(process.env.LOCALLM_XVFB_BIN || 'Xvfb', ['-displayfd', '1', '-screen', '0', '1440x1000x24', '-nolisten', 'tcp', '-ac', '-noreset'], { stdio: ['ignore', 'pipe', 'inherit'] });
let driver;
const log = await open(resolve(output, 'native.log'), 'w');
try {
  const number = await new Promise((done, fail) => {
    const timer = setTimeout(() => fail(new Error('Xvfb startup timeout')), 10000);
    const lines = createInterface({ input: display.stdout });
    display.once('error', error => { clearTimeout(timer); fail(error); });
    lines.once('line', line => { clearTimeout(timer); lines.close(); /^\d+$/.test(line) ? done(line) : fail(new Error('Invalid display')); });
  });
  const env = { ...process.env, DISPLAY: `:${number}`, WAYLAND_DISPLAY: '', GDK_BACKEND: 'x11', XDG_SESSION_TYPE: 'x11', XDG_DATA_HOME: resolve(output, 'data'), XDG_CONFIG_HOME: resolve(output, 'config'), XDG_CACHE_HOME: resolve(output, 'cache'), LOCALLM_DESKTOP_TEST_ISOLATED: '1', NO_AT_BRIDGE: '0', GTK_MODULES: 'gail:atk-bridge', LIBGL_ALWAYS_SOFTWARE: '1', WEBKIT_DISABLE_COMPOSITING_MODE: '1' };
  driver = spawn('dbus-run-session', ['--', '/usr/bin/python3', resolve('scripts/native-coding-agent-acceptance.py'), binary, output], { env, detached: true, stdio: ['ignore', log.fd, log.fd] });
  await new Promise((done, fail) => {
    const timer = setTimeout(() => fail(new Error('Native coding-agent acceptance timeout')), 180000);
    driver.once('error', error => { clearTimeout(timer); fail(error); });
    driver.once('exit', code => { clearTimeout(timer); code === 0 ? done() : fail(new Error(`Native acceptance failed (${code}); see ${output}/native.log`)); });
  });
  if (sha256(await readFile(binary)) !== provenance.binarySha256) throw new Error('Native binary changed during acceptance');
  console.log(`Native workflow captures: ${output}`);
} finally {
  if (driver?.pid) { try { process.kill(-driver.pid, 'SIGTERM'); } catch {} }
  display.kill('SIGTERM');
  await log.close();
}
