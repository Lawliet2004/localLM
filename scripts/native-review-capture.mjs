// Launch the real application with a fresh data/config directory and its own
// X11 and D-Bus sessions. No user database, workspace or desktop is touched.
import { spawn } from 'node:child_process';
import { mkdir, open } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline';

if (process.platform !== 'linux') throw new Error('This native smoke launcher requires Linux.');
const output = resolve(process.env.LOCALLM_SMOKE_OUTPUT || '.local/qa/native-app');
await mkdir(output, { recursive: true });
const run = (command, args, env = process.env) => new Promise((resolveRun, reject) => {
  const child = spawn(command, args, { env, stdio: 'inherit' });
  child.once('error', reject);
  child.once('exit', code => code === 0 ? resolveRun() : reject(new Error(`${command} exited ${code}`)));
});
await run('cargo', ['build', '--manifest-path', 'src-tauri/Cargo.toml', '--example', 'review_capture_smoke', '--bin', 'locallm']);
// A debug Tauri binary uses the configured Vite URL.
const response = await fetch('http://127.0.0.1:1420', { signal: AbortSignal.timeout(5000) });
if (!response.ok) throw new Error('Start npm run dev before the native debug smoke test.');
const display = spawn(process.env.LOCALLM_XVFB_BIN || 'Xvfb', ['-displayfd', '1', '-screen', '0', '1440x1000x24', '-nolisten', 'tcp', '-ac', '-noreset'], { stdio: ['ignore', 'pipe', 'inherit'] });
let app;
const log = await open(resolve(output, 'native-app.log'), 'w');
try {
  const number = await new Promise((resolveDisplay, reject) => {
    const timer = setTimeout(() => reject(new Error('Xvfb startup timed out')), 10000);
    const lines = createInterface({ input: display.stdout });
    display.once('error', error => { clearTimeout(timer); reject(error); });
    lines.once('line', line => { clearTimeout(timer); lines.close(); /^\d+$/.test(line) ? resolveDisplay(line) : reject(new Error('Invalid Xvfb display')); });
  });
  const env = { ...process.env, DISPLAY: `:${number}`, WAYLAND_DISPLAY: '', GDK_BACKEND: 'x11', XDG_SESSION_TYPE: 'x11', XDG_DATA_HOME: resolve(output, 'data'), XDG_CONFIG_HOME: resolve(output, 'config'), XDG_CACHE_HOME: resolve(output, 'cache'), LOCALLM_DESKTOP_TEST_ISOLATED: '1', LIBGL_ALWAYS_SOFTWARE: '1', WEBKIT_DISABLE_COMPOSITING_MODE: '1' };
  app = spawn('dbus-run-session', ['--', resolve('src-tauri/target/debug/locallm')], { env, detached: true, stdio: ['ignore', log.fd, log.fd] });
  let windowId;
  for (let attempt = 0; attempt < 60; attempt++) {
    if (app.exitCode !== null || app.signalCode !== null) throw new Error('Native app exited before creating a window. Inspect native-app.log.');
    const windows = await new Promise(resolveWindows => {
      const child = spawn('xwininfo', ['-root', '-tree'], { env }); let text = '';
      child.stdout.on('data', chunk => { text += chunk; });
      child.once('exit', () => resolveWindows(text));
    });
    for (const found of windows.matchAll(/(0x[0-9a-f]+) "LocalLM"/gi)) {
      const detail = await new Promise(resolveDetail => {
        const child = spawn('xwininfo', ['-id', found[1]], { env }); let text = '';
        child.stdout.on('data', chunk => { text += chunk; }); child.once('exit', () => resolveDetail(text));
      });
      if (detail.includes('Map State: IsViewable')) { windowId = found[1]; break; }
    }
    if (windowId) break;
    await new Promise(resolveDelay => setTimeout(resolveDelay, 250));
  }
  if (!windowId) throw new Error('Native app did not create its main window.');
  await run('src-tauri/target/debug/examples/review_capture_smoke', [output, windowId], env);
  if (app.exitCode !== null) throw new Error('Native app exited during interaction.');
  console.log(`Native smoke captures saved under ${output}`);
} finally {
  if (app?.pid) {
    try { process.kill(-app.pid, 'SIGTERM'); } catch {}
    await new Promise(resolveDelay => setTimeout(resolveDelay, 250));
    try { process.kill(-app.pid, 'SIGKILL'); } catch {}
  }
  if (display.exitCode === null && display.signalCode === null) { display.kill('SIGTERM'); await new Promise(resolveExit => display.once('exit', resolveExit)); }
  await log.close();
}
