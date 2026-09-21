// Agent-quality eval runner (docs/EXTENSIONS.md §2.5).
//
// Drives a *running debug* LocalLM app over WebView2 CDP, exactly like the
// smoke scripts: launch the debug binary with
//   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223
// and load the model you want to measure first. Every task runs in a fresh
// copy of its fixture workspace, through the real agent loop, approval path
// and session log; nothing is simulated.
//
//   node scripts/eval/run.mjs [--tasks evals/tasks] [--only id,id]
//     [--presets standard,coding] [--repeats 3] [--seed 1234|none]
//     [--timeout 600] [--out test-results/eval] [--label name] [--cleanup]
import { chromium } from '@playwright/test';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { evaluateAssertions, scoreTrajectory, validateTask } from './score.mjs';
import { renderMarkdown } from './report.mjs';

function parseArgs(argv) {
  const options = { tasks: 'evals/tasks', fixtures: 'evals/fixtures', only: null, presets: ['standard'], repeats: 3, seed: 1234, timeout: 600, out: 'test-results/eval', label: null, cleanup: false, cdp: 'http://127.0.0.1:9223' };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = () => { const next = argv[++index]; if (next === undefined) throw new Error(`${flag} needs a value`); return next; };
    switch (flag) {
      case '--tasks': options.tasks = value(); break;
      case '--fixtures': options.fixtures = value(); break;
      case '--only': options.only = value().split(','); break;
      case '--presets': options.presets = value().split(','); break;
      case '--repeats': options.repeats = Number(value()); break;
      case '--seed': { const seed = value(); options.seed = seed === 'none' ? null : Number(seed); break; }
      case '--timeout': options.timeout = Number(value()); break;
      case '--out': options.out = value(); break;
      case '--label': options.label = value(); break;
      case '--cdp': options.cdp = value(); break;
      case '--cleanup': options.cleanup = true; break;
      default: throw new Error(`Unknown option ${flag}`);
    }
  }
  if (!Number.isInteger(options.repeats) || options.repeats < 1) throw new Error('--repeats must be a positive integer');
  if (options.seed !== null && !(Number.isInteger(options.seed) && options.seed >= 0 && options.seed <= 0xffffffff)) throw new Error('--seed must be an integer 0..4294967295 or "none"');
  if (!(options.timeout > 0)) throw new Error('--timeout must be positive seconds');
  return options;
}

function loadTasks(dir, only) {
  const tasks = readdirSync(dir).filter(file => file.endsWith('.json')).sort().map(file => {
    const path = join(dir, file);
    return validateTask(JSON.parse(readFileSync(path, 'utf8')), path);
  });
  const ids = new Set();
  for (const task of tasks) { if (ids.has(task.id)) throw new Error(`Duplicate task id ${task.id}`); ids.add(task.id); }
  if (!only) return tasks;
  const missing = only.filter(id => !ids.has(id));
  if (missing.length) throw new Error(`Unknown task id(s): ${missing.join(', ')}`);
  return tasks.filter(task => only.includes(task.id));
}

function readText(path) {
  try { return readFileSync(path, 'utf8'); } catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}

const options = parseArgs(process.argv.slice(2));
const tasks = loadTasks(options.tasks, options.only);
const workRoot = mkdtempSync(join(tmpdir(), 'locallm-eval-'));

const browser = await chromium.connectOverCDP(options.cdp);
const page = browser.contexts()[0]?.pages().find(value => value.url().includes('1420'));
if (!page) throw new Error('Native LocalLM webview is unavailable. Launch the debug app with WebView2 CDP on 9223.');
const invoke = (command, args = {}) => page.evaluate(async ([command, args]) => {
  const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
  return invoke(command, args);
}, [command, args]);

const boot = await invoke('bootstrap');
const selection = boot.preferredModel ?? { providerId: null, modelId: '' };
const local = !selection.providerId;
if (local && boot.runtime.phase !== 'ready') throw new Error('No local model is loaded. Load the model to evaluate in Models & runtime first.');
const model = options.label ?? (local ? basename(boot.runtime.modelPath ?? boot.preferences.modelPath) : `${selection.providerId}/${selection.modelId}`);
const knownPresets = (await invoke('list_presets')).map(preset => preset.id);
const unknownPresets = options.presets.filter(preset => !knownPresets.includes(preset));
if (unknownPresets.length) throw new Error(`Unknown preset(s): ${unknownPresets.join(', ')}. Known: ${knownPresets.join(', ')}`);

const originalPreferences = boot.preferences;
const originalWorkspace = (await invoke('get_workspace').catch(() => null))?.path ?? null;
const report = {
  startedAt: new Date().toISOString(),
  model,
  backend: local ? 'local' : selection.providerId,
  runtimeConfig: local ? boot.runtime.loadedConfig ?? boot.config : null,
  seed: options.seed,
  // A seed only makes the local llama.cpp sampler reproducible; remote APIs
  // treat it as best-effort or drop it (inference.rs).
  seedReproducible: options.seed !== null && local,
  repeats: options.repeats,
  presets: options.presets,
  runs: [],
  skipped: [],
};

// Run one turn inside the webview so the Tauri Channel can answer approvals
// as they arrive. Policy: approve listed tools, deny the rest; ask_user is
// always dismissed so an unattended run cannot hang.
async function runTurn(conversationId, content, approve, timeoutMs) {
  return page.evaluate(async ({ conversationId, content, approve, timeoutMs }) => {
    const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
    const handled = new Set();
    const approvals = [];
    const contexts = [];
    let answer = '';
    const channel = new Channel();
    channel.onmessage = event => {
      if (event.context && Number.isFinite(event.context.inputTokens)) contexts.push(event.context);
      const approval = event.approval;
      if (!approval?.id || handled.has(approval.id)) return;
      handled.add(approval.id);
      if (approval.kind === 'ask_user') {
        approvals.push({ name: 'ask_user', allowed: false });
        invoke('resolve_ask_user', { id: approval.id, choice: null }).catch(() => {});
        return;
      }
      const allowed = approve === 'all' || (Array.isArray(approve) && approve.includes(approval.name));
      approvals.push({ name: approval.name, allowed });
      invoke('resolve_tool_approval', { id: approval.id, allow: allowed }).catch(() => {});
    };
    let timedOut = false;
    let error = null;
    const started = performance.now();
    const timer = setTimeout(() => { timedOut = true; invoke('cancel_generation').catch(() => {}); }, timeoutMs);
    try {
      await invoke('send_message', { conversationId, content, connectorIds: null, connectorTools: null, planMode: false, channel });
    } catch (caught) {
      error = String(caught);
    } finally {
      clearTimeout(timer);
    }
    const wallMs = performance.now() - started;
    const messages = await invoke('get_messages', { id: conversationId });
    const last = [...messages].reverse().find(message => message.role === 'assistant');
    answer = last?.content ?? '';
    return { wallMs, approvals, contexts, timedOut, error, answer };
  }, { conversationId, content, approve, timeoutMs });
}

async function sessionEvents(conversationId) {
  const events = [];
  for (let fromSeq = 0; ;) {
    const batch = await invoke('get_session_events', { conversationId, fromSeq, limit: 5000 });
    events.push(...batch);
    if (batch.length < 5000) return events;
    fromSeq = batch[batch.length - 1].seq + 1;
  }
}

mkdirSync(options.out, { recursive: true });
const stamp = report.startedAt.replace(/[:.]/g, '-');
try {
  await invoke('save_preferences', { preferences: { ...originalPreferences, sampling: { ...(originalPreferences.sampling ?? {}), seed: options.seed ?? undefined } } });
  for (const preset of options.presets) {
    for (const task of tasks) {
      if (task.presets && !task.presets.includes(preset)) {
        report.skipped.push({ taskId: task.id, preset, reason: `task is limited to presets: ${task.presets.join(', ')}` });
        continue;
      }
      for (let repeat = 1; repeat <= options.repeats; repeat += 1) {
        const workspace = join(workRoot, `${task.id}-${preset}-${repeat}`);
        if (task.fixture) {
          const source = resolve(options.fixtures, task.fixture);
          if (!existsSync(source)) throw new Error(`${task.id}: fixture ${task.fixture} not found in ${options.fixtures}`);
          cpSync(source, workspace, { recursive: true });
        } else {
          mkdirSync(workspace, { recursive: true });
        }
        await invoke('set_workspace', { path: workspace });
        const conversation = await invoke('create_conversation');
        await invoke('rename_conversation', { id: conversation.id, title: `[eval] ${task.id} · ${preset} · #${repeat}` });
        await invoke('set_preset', { conversationId: conversation.id, preset: preset });
        const observed = await runTurn(conversation.id, task.prompt, task.approve ?? 'none', (task.timeout ?? options.timeout) * 1000);
        const events = await sessionEvents(conversation.id);
        const metrics = scoreTrajectory(events, observed);
        const assertions = evaluateAssertions(task.assertions, { answer: observed.answer, metrics, workspace, readFile: readText });
        const passed = assertions.every(assertion => assertion.pass);
        const run = { model, preset: preset, taskId: task.id, repeat, conversationId: conversation.id, workspace, passed, metrics, assertions, answer: observed.answer, approvalLog: observed.approvals };
        report.runs.push(run);
        console.log(`${passed ? 'PASS' : 'FAIL'} ${task.id} [${run.preset} #${repeat}] ${(metrics.wallMs / 1000).toFixed(1)}s tools=${metrics.toolCalls} status=${metrics.status}`);
        if (options.cleanup) await invoke('delete_conversation', { id: conversation.id });
        writeFileSync(join(options.out, `eval-${stamp}.json`), JSON.stringify(report, null, 2));
      }
    }
  }
} finally {
  // Restore the user's own settings even when a run fails midway.
  await invoke('save_preferences', { preferences: originalPreferences }).catch(error => console.error(`Could not restore preferences: ${error}`));
  if (typeof originalWorkspace === 'string') await invoke('set_workspace', { path: originalWorkspace }).catch(error => console.error(`Could not restore workspace: ${error}`));
  await browser.close();
}

writeFileSync(join(options.out, `eval-${stamp}.json`), JSON.stringify(report, null, 2));
writeFileSync(join(options.out, `eval-${stamp}.md`), renderMarkdown(report));
console.log(`\nReport: ${join(options.out, `eval-${stamp}.md`)}`);
