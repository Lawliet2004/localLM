// Isolated embedded Tauri review. No Vite server, browser, or proxy is used.
import { spawn, execFileSync } from 'node:child_process';
import { mkdir, open, readFile, writeFile, access } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { createHash } from 'node:crypto';
const output = resolve(process.argv[2] || `.local/qa/response-review-${Date.now()}`);
const binary = resolve(process.env.LOCALLM_NATIVE_BINARY || 'src-tauri/target/debug/locallm');
try { await access(resolve(output, 'data')); throw Error('Use a fresh output directory; existing review data is preserved.'); } catch (error) { if (error.code !== 'ENOENT') throw error; }
const files = [...new Set(execFileSync('git', ['ls-files', '-co', '--exclude-standard', 'src', 'src-tauri/src', 'src-tauri/tauri.conf.json'], { encoding: 'utf8' }).trim().split('\n').filter(Boolean))].sort();
async function sourceHash() { const hash = createHash('sha256'); for (const path of files) { hash.update(path); hash.update(await readFile(path)); } return hash.digest('hex'); }
const beforeSource = await sourceHash();
const binaryHash = createHash('sha256').update(await readFile(binary)).digest('hex');
await mkdir(output, { recursive: true });
const display = spawn(process.env.LOCALLM_XVFB_BIN || 'Xvfb', ['-displayfd', '1', '-screen', '0', '1440x1000x24', '-nolisten', 'tcp', '-ac', '-noreset'], { stdio: ['ignore', 'pipe', 'inherit'] });
let app;
let displayError;
display.on('error', error => { displayError = error; });
const log = await open(resolve(output, 'launcher.log'), 'w');
try {
  if (displayError) throw displayError;
  const number = await new Promise((done, reject) => {
    const timer = setTimeout(() => reject(Error('Display timeout')), 10000);
    const lines = createInterface({ input: display.stdout });
    lines.once('line', line => { clearTimeout(timer); lines.close(); done(line); });
    display.once('error', reject);
  });
  const env = { ...process.env, DISPLAY: `:${number}`, WAYLAND_DISPLAY: '', GDK_BACKEND: 'x11', XDG_SESSION_TYPE: 'x11', XDG_DATA_HOME: resolve(output, 'data'), XDG_CONFIG_HOME: resolve(output, 'config'), XDG_CACHE_HOME: resolve(output, 'cache'), LOCALLM_EMBEDDED_REVIEW: '1', LOCALLM_DESKTOP_TEST_ISOLATED: '1', LOCALLM_REVIEW_RICH: '1', NO_AT_BRIDGE: '0', GTK_MODULES: 'gail:atk-bridge', LIBGL_ALWAYS_SOFTWARE: '1', WEBKIT_DISABLE_COMPOSITING_MODE: '1' };
  app = spawn('dbus-run-session', ['--', '/usr/bin/python3', resolve('scripts/native-embedded-conversation-review.py'), binary, output], { env, detached: true, stdio: ['ignore', log.fd, log.fd] });
  const code = await new Promise((done, reject) => { const timer = setTimeout(() => reject(Error('Native review timeout')), 180000); app.once('exit', code => { clearTimeout(timer); done(code); }); app.once('error', reject); });
  if (code !== 0) throw Error(`Native review exited ${code}; inspect ${output}/launcher.log`);
  const afterSource = await sourceHash();
  if (afterSource !== beforeSource) throw Error('Source changed during review; rerun against the finished tree.');
  await writeFile(resolve(output, 'provenance.json'), JSON.stringify({ capturedAt: new Date().toISOString(), sourceSha256: beforeSource, binarySha256: binaryHash, fixtureSha256: createHash('sha256').update(await readFile('scripts/native-embedded-conversation-review.py')).digest('hex'), width: Number(process.env.LOCALLM_REVIEW_WIDTH || 1280), height: Number(process.env.LOCALLM_REVIEW_HEIGHT || 840), theme: process.env.LOCALLM_REVIEW_THEME || 'light', nativeEmbedded: true, browserPreview: false, paidInference: false, syntheticTranscript: true }, null, 2));
} finally {
  if (app?.pid) { try { process.kill(-app.pid, 'SIGTERM'); } catch {} }
  display.kill('SIGTERM'); await log.close();
}
