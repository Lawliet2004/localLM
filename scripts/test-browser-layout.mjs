// Exercise the real WebKitGTK child views without interacting with the user's
// desktop, workspace, database, model runtime, or credential store.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';

if (process.platform !== 'linux') throw new Error('This native layout check requires Linux and Xvfb.');
const run = (command, args, env = process.env) => new Promise((resolve, reject) => {
  const child = spawn(command, args, { env, stdio: 'inherit' });
  const timer = setTimeout(() => { child.kill('SIGKILL'); reject(new Error(`${command} timed out.`)); }, 60000);
  child.once('error', error => { clearTimeout(timer); reject(error); });
  child.once('exit', (code, signal) => { clearTimeout(timer); code === 0 ? resolve() : reject(new Error(`${command} exited ${code ?? signal}`)); });
});
await run('cargo', ['build', '--manifest-path', 'src-tauri/Cargo.toml', '--example', 'browser_layout_smoke']);
const display = spawn(process.env.LOCALLM_XVFB_BIN || 'Xvfb', ['-displayfd', '1', '-screen', '0', '1280x800x24', '-nolisten', 'tcp', '-ac', '-noreset'], { stdio: ['ignore', 'pipe', 'inherit'] });
try {
  const number = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Xvfb startup timed out.')), 10000);
    const lines = createInterface({ input: display.stdout });
    display.once('error', error => { clearTimeout(timer); lines.close(); reject(error); });
    display.once('exit', () => { clearTimeout(timer); lines.close(); reject(new Error('Xvfb exited before startup.')); });
    lines.once('line', line => { clearTimeout(timer); lines.close(); /^\d+$/.test(line) ? resolve(line) : reject(new Error('Invalid display number.')); });
  });
  await run('src-tauri/target/debug/examples/browser_layout_smoke', [], {
    ...process.env, DISPLAY: `:${number}`, WAYLAND_DISPLAY: '', GDK_BACKEND: 'x11', XDG_SESSION_TYPE: 'x11',
    LOCALLM_BROWSER_TEST_ISOLATED: '1',
  });
} finally {
  if (display.exitCode === null && display.signalCode === null) {
    display.kill('SIGTERM'); await new Promise(resolve => display.once('exit', resolve));
  }
}
