// Real native input runs only in the Xvfb instance owned by this script.
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';

if (process.platform !== 'linux') throw new Error('This isolated desktop check requires Linux and Xvfb.');
const directory = await mkdtemp(join(tmpdir(), 'locallm-desktop-test-'));
const fixture = join(directory, 'fixture');
const browser = process.argv.includes('--browser');
const run = (command, args, options = {}) => new Promise((resolveRun, reject) => {
  const child = spawn(command, args, { stdio: 'inherit', ...options });
  child.on('error', reject); child.on('exit', (code, signal) => code === 0 ? resolveRun() : reject(new Error(`${command} exited ${code ?? signal}`)));
});
let display;
try {
  if (!browser) await run(process.env.CC || 'cc', [resolve('scripts/fixtures/automation-desktop.c'), '-lX11', '-o', fixture]);
  display = spawn(process.env.LOCALLM_XVFB_BIN || 'Xvfb', ['-displayfd', '1', '-screen', '0', '800x600x24', '-nolisten', 'tcp', '-ac', '-noreset'], { stdio: ['ignore', 'pipe', 'inherit'] });
  const number = await new Promise((resolveDisplay, reject) => {
    const timer = setTimeout(() => reject(new Error('Xvfb did not become ready.')), 10000);
    const lines = createInterface({ input: display.stdout });
    display.once('error', error => { clearTimeout(timer); lines.close(); reject(error); });
    display.once('exit', () => { clearTimeout(timer); lines.close(); reject(new Error('Xvfb exited before readiness.')); });
    lines.once('line', line => { clearTimeout(timer); lines.close(); /^\d+$/.test(line) ? resolveDisplay(line) : reject(new Error('Invalid Xvfb display number.')); });
  });
  await run('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--lib', browser ? 'automation::tests::real_browser_form_tabs_screenshots_isolation_and_cleanup' : 'computer_use::tests::isolated_x11_driver_controls_fixture', '--', '--ignored', '--exact', '--nocapture'], {
    env: { ...process.env, DISPLAY: `:${number}`, XDG_SESSION_TYPE: 'x11', WAYLAND_DISPLAY: '', LOCALLM_DESKTOP_TEST_ISOLATED: '1', LOCALLM_DESKTOP_TEST_FIXTURE: fixture, LOCALLM_BROWSER_TEST_HEADED: browser ? '1' : '0' },
  });
} finally {
  if (display?.pid && display.exitCode === null && display.signalCode === null) { display.kill('SIGTERM'); await new Promise(resolveExit => display.once('exit', resolveExit)); }
  await rm(directory, { recursive: true, force: true });
}
