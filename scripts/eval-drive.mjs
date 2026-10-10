// Drives LocalLM's own harness end-to-end over WebView2 CDP:
//   node scripts/eval-drive.mjs setup
//   node scripts/eval-drive.mjs run --model arex --tests 1,4,7
//   node scripts/eval-drive.mjs run --model minicpm --tests all
// Results are written to eval-results/<model>/<unique-run-id>/.
// Requires the debug app running with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223
// and the Vite dev server on :1420.
import { chromium } from '@playwright/test';
import { existsSync, mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { resolve } from 'node:path';
import { TESTS } from './eval-tests.mjs';
import { evaluateKnownAnswer } from './eval-oracles.mjs';
import {
  copyRepoSnapshot,
  createEvalRunDirectory,
  requireReadyEvaluationModel,
  restoreWorkspace,
  withWorkspace,
  workspaceForTest,
} from './eval-safety.mjs';

const CDP = process.env.LOCALLM_CDP_URL ?? 'http://127.0.0.1:9223';
const REPO = resolve('.');
const SCRATCH = resolve('../locallm-eval-ws');
const OUT = resolve('eval-results');
const ASK_USER_ANSWER = 'Proceed autonomously using your best judgment; do not ask again.';

function buildIdentity() {
  const revision = (() => {
    try { return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: REPO, encoding: 'utf8' }).trim(); }
    catch { return null; }
  })();
  const workerPath = resolve(REPO, 'src-tauri/resources/web/worker.mjs');
  const workingTreeDirty = (() => {
    try { return execFileSync('git', ['status', '--porcelain', '--untracked-files=normal'], { cwd: REPO, encoding: 'utf8' }).trim().length > 0; }
    catch { return null; }
  })();
  return {
    revision,
    workingTreeDirty,
    workerSha256: existsSync(workerPath) ? createHash('sha256').update(readFileSync(workerPath)).digest('hex') : null,
  };
}

const arg = (name, fallback) => {
  const idx = process.argv.indexOf(`--${name}`);
  if (idx !== -1 && process.argv[idx + 1] && !process.argv[idx + 1].startsWith('--')) return process.argv[idx + 1];
  const hit = process.argv.find(a => a.startsWith(`--${name}=`));
  return hit ? hit.split('=').slice(1).join('=') : fallback;
};
const sleep = ms => new Promise(r => setTimeout(r, ms));

async function connect() {
  const browser = await chromium.connectOverCDP(CDP);
  const page = browser.contexts()[0].pages().find(p => p.url().includes('1420'));
  if (!page) throw new Error('LocalLM webview not found on CDP.');
  return { browser, page };
}

const inv = (page, cmd, args = {}) => page.evaluate(async ({ cmd, args }) => {
  const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
  return invoke(cmd, args);
}, { cmd, args });

async function findModelId(page, needle) {
  const models = await inv(page, 'list_installed_models');
  const m = models.find(m => m.filename.toLowerCase().includes(needle.toLowerCase()) && m.complete);
  if (!m) throw new Error(`No complete installed model matching '${needle}'. Have: ${models.map(x => x.filename).join(', ')}`);
  return m;
}

async function ensureModel(page, needle) {
  const [status, model] = await Promise.all([
    inv(page, 'runtime_status'),
    findModelId(page, needle),
  ]);
  return requireReadyEvaluationModel(status, model);
}

async function runTest(page, test, modelNeedle, ctx = {}, repoSnapshot = null, savedWorkspacePath) {
  const model = await ensureModel(page, modelNeedle);
  const ws = workspaceForTest(test, SCRATCH, repoSnapshot);
  mkdirSync(ws, { recursive: true });
  return withWorkspace(
    (command, args) => inv(page, command, args),
    ws,
    () => runTestInWorkspace(page, test, model, ws, ctx),
    savedWorkspacePath,
  );
}

async function runTestInWorkspace(page, test, model, ws, ctx) {
  const repoTest = test.workspace === 'repo';
  const connectorIds = repoTest ? ['__workspace'] : ['__workspace', '__execution'];
  const convo = await inv(page, 'create_conversation', { projectId: null });
  ctx.convoId = convo.id;
  await inv(page, 'set_preset', { conversationId: convo.id, preset: repoTest ? 'research' : 'creator' });
  await inv(page, 'save_conversation_tools', {
    id: convo.id,
    tools: { sources: connectorIds, tools: [], accessMode: 'fullAccess' },
  });

  await page.evaluate(({ conversationId, content, timeoutMs, connectorIds }) => {
    window.__eval = { events: [], resolved: [], done: null };
    // In-page watchdog: cancel the run at the deadline even if Node-side IPC
    // is stalled behind the agent loop's store lock.
    window.__eval.timer = setTimeout(() => {
      import('/node_modules/@tauri-apps/api/core.js').then(({ invoke }) => invoke('cancel_generation')).catch(() => {});
    }, timeoutMs);
    (async () => {
      const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
      const channel = new Channel();
      channel.onmessage = e => window.__eval.events.push(e);
      try {
        await invoke('send_message', { conversationId, content, channel, connectorIds, connectorTools: [], planMode: false });
        window.__eval.done = 'ok';
      } catch (e) { window.__eval.done = 'err: ' + String(e); }
      clearTimeout(window.__eval.timer);
    })();
  }, { conversationId: convo.id, content: test.prompt, timeoutMs: test.timeoutMin * 60_000, connectorIds });

  const started = Date.now();
  // Grace period past the in-page watchdog deadline for the run to settle.
  const deadline = started + (test.timeoutMin + 3) * 60_000;
  let timedOut = false;
  while (Date.now() < deadline) {
    await sleep(4000);
    // auto-resolve any pending approvals (ask_user under fullAccess, stray tool approvals)
    await page.evaluate(async (answer) => {
      const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
      for (const e of window.__eval.events) {
        const ap = e.approval;
        if (ap && ap.id && !window.__eval.resolved.includes(ap.id)) {
          window.__eval.resolved.push(ap.id);
          try {
            if (ap.kind === 'ask_user') {
              const opts = Array.isArray(ap.arguments?.options) ? ap.arguments.options.filter(o => typeof o === 'string') : [];
              await invoke('resolve_ask_user', { id: ap.id, choice: opts[0] ?? answer });
            } else {
              await invoke('resolve_tool_approval', { id: ap.id, allow: true });
            }
          } catch (e) { window.__eval.events.push({ notice: 'resolve-failed: ' + String(e) }); }
        }
      }
    }, ASK_USER_ANSWER).catch(() => {});
    const done = await page.evaluate(() => window.__eval.done).catch(() => 'page-lost');
    if (done) break;
    const run = await inv(page, 'get_conversation_run', { conversationId: convo.id }).catch(() => null);
    if (run && ['completed', 'failed', 'cancelled', 'outcome_unknown'].includes(run.status)) {
      // wait briefly for send_message promise to settle
      for (let i = 0; i < 15 && !(await page.evaluate(() => window.__eval.done).catch(() => 'x')); i++) await sleep(1000);
      break;
    }
  }
  if (!(await page.evaluate(() => window.__eval.done).catch(() => null))) {
    timedOut = true;
    await inv(page, 'cancel_generation').catch(() => {});
    for (let i = 0; i < 30 && !(await page.evaluate(() => window.__eval.done).catch(() => null)); i++) await sleep(2000);
  }

  const events = await page.evaluate(() => window.__eval?.events ?? []).catch(() => []);
  const messages = await inv(page, 'get_messages', { id: convo.id }).catch(e => [{ error: String(e) }]);
  const runs = await inv(page, 'get_conversation_runs', { conversationId: convo.id }).catch(() => []);
  const runEvents = [];
  for (const r of runs) runEvents.push(...await inv(page, 'get_run_events', { runId: r.id }).catch(() => []));
  const sessionEvents = await inv(page, 'get_session_events', { conversationId: convo.id, fromSeq: null, limit: null }).catch(() => []);
  const todos = await inv(page, 'get_todos', { conversationId: convo.id }).catch(() => []);
  const goal = await inv(page, 'get_goal', { conversationId: convo.id }).catch(() => null);
  const runtimeStatus = await inv(page, 'runtime_status').catch(() => null);

  return {
    testId: test.id, slug: test.slug, model: model.filename, modelId: model.id,
    build: buildIdentity(),
    conversationId: convo.id, workspace: ws, startedAt: new Date(started).toISOString(),
    elapsedMs: Date.now() - started, timedOut,
    chatEvents: events, messages, messageMetrics: messages.filter(message => message.metrics).map(message => ({ id: message.id, metrics: message.metrics })),
    runs, runEvents, sessionEvents, todos, goal, runtimeStatus,
    checks: evaluateKnownAnswer(test.id, messages),
  };
}

const cmd = process.argv[2];

if (cmd === 'setup') {
  const { browser, page } = await connect();
  const [configuredExecution, detectedInterpreters, webConfig] = await Promise.all([
    inv(page, 'get_execution_config'),
    inv(page, 'detect_interpreters'),
    inv(page, 'get_web_search_config'),
  ]);
  console.log('configured_execution:', JSON.stringify(configuredExecution));
  console.log('detected_interpreters:', JSON.stringify(detectedInterpreters));
  console.log('web_search_config:', JSON.stringify({
    provider: webConfig.provider,
    searxngBaseUrl: webConfig.searxngBaseUrl,
    searchFallbackEnabled: webConfig.searchFallbackEnabled,
    googleApiKeyConfigured: Boolean(webConfig.googleApiKey),
    googleCxIdConfigured: Boolean(webConfig.googleCxId),
  }));
  const health = await inv(page, 'web_search_health').catch(e => ({ error: String(e) }));
  console.log('web_search_health:', JSON.stringify(health).slice(0, 2000));
  const models = await inv(page, 'list_installed_models');
  console.log('models:', models.map(m => `${m.filename} complete=${m.complete} id=${m.id}`).join('\n  '));
  const caps = await inv(page, 'list_capabilities', { conversationId: null });
  console.log('capabilities:', caps.map(c => `${c.id}=${c.enabled}`).join(' '));
  await browser.close();
} else if (cmd === 'run') {
  const modelNeedle = arg('model', 'MiniCPM');
  const testsArg = arg('tests', 'all');
  const ids = testsArg === 'all' ? TESTS.map(t => t.id) : testsArg.split(',').flatMap(p => {
    if (p.includes('-')) { const [a, b] = p.split('-').map(Number); return Array.from({ length: b - a + 1 }, (_, i) => a + i); }
    return [Number(p)];
  });
  const slug = modelNeedle.toLowerCase().replace(/[^a-z0-9]+/g, '-');
  let { browser, page } = await connect();
  const savedWorkspacePath = (await inv(page, 'get_workspace')).path ?? '';
  const runId = `${new Date().toISOString().replace(/[^a-z0-9]+/gi, '-')}-${process.pid}-${randomUUID()}`;
  const dir = createEvalRunDirectory(OUT, slug || 'model', runId);
  console.log('evaluation output:', dir);
  let failedChecks = 0;
  page.on('pageerror', e => console.error('[pageerror]', e.message));
  tests: for (const id of ids) {
    const test = TESTS.find(t => t.id === id);
    if (!test) { console.error(`unknown test ${id}`); process.exitCode = 1; continue; }
    const outFile = resolve(dir, `${String(id).padStart(2, '0')}-${test.slug}.json`);
    console.log(`>>> test ${id} ${test.slug} on ${modelNeedle} — ${new Date().toISOString()}`);
    const ctx = {};
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        const repoSnapshot = test.workspace === 'repo'
          ? copyRepoSnapshot(REPO, resolve(SCRATCH, `repo-snapshot-${runId}-${id}-${attempt}`))
          : null;
        const result = await runTest(page, test, modelNeedle, ctx, repoSnapshot, savedWorkspacePath);
        writeFileSync(outFile, JSON.stringify(result, null, 2));
        console.log(`<<< ${id} done elapsed=${(result.elapsedMs / 60000).toFixed(1)}min timedOut=${result.timedOut} msgs=${result.messages.length} events=${result.chatEvents.length} runs=${result.runs.map(r => r.status).join(',')}`);
        for (const [name, check] of Object.entries(result.checks)) {
          console.log(`    check ${name}: ${check.verdict}`);
          if (check.verdict !== 'pass') failedChecks += 1;
        }
        break;
      } catch (e) {
        const lost = /closed|crash|destroyed/i.test(String(e));
        if (!lost || attempt === 2) {
          const failure = { testId: id, slug: test.slug, model: modelNeedle, error: String(e), at: new Date().toISOString() };
          if (lost) failure.workspaceRestoreRequired = savedWorkspacePath;
          writeFileSync(outFile, JSON.stringify(failure, null, 2));
          console.error(`<<< ${id} ERROR: ${e}`);
          process.exitCode = 1;
          if (lost) break tests;
          break;
        }
        console.error(`<<< ${id} target lost, reconnecting (attempt ${attempt + 2})…`);
        browser = null;
        page = null;
        for (let i = 0; i < 30; i++) {
          try { ({ browser, page } = await connect()); break; } catch { await sleep(5000); }
        }
        if (!page) {
          writeFileSync(outFile, JSON.stringify({
            testId: id, slug: test.slug, model: modelNeedle,
            error: 'webview lost and reconnect failed',
            workspaceRestoreRequired: savedWorkspacePath,
            at: new Date().toISOString(),
          }, null, 2));
          process.exitCode = 1;
          break tests;
        }
        page.on('pageerror', e => console.error('[pageerror]', e.message));
        try {
          await restoreWorkspace((command, args) => inv(page, command, args), savedWorkspacePath);
        } catch (restoreError) {
          writeFileSync(outFile, JSON.stringify({
            testId: id, slug: test.slug, model: modelNeedle,
            error: `webview reconnected but saved workspace restoration failed: ${restoreError}`,
            workspaceRestoreRequired: savedWorkspacePath,
            at: new Date().toISOString(),
          }, null, 2));
          process.exitCode = 1;
          break tests;
        }
        await sleep(5000);
      }
    }
  }
  if (page) {
    try {
      await restoreWorkspace((command, args) => inv(page, command, args), savedWorkspacePath);
    } catch (error) {
      process.exitCode = 1;
      const recovery = {
        savedWorkspacePath,
        error: String(error),
        at: new Date().toISOString(),
      };
      writeFileSync(resolve(dir, 'workspace-restore-required.json'), JSON.stringify(recovery, null, 2));
      console.error(`failed to restore the original workspace; recovery details: ${resolve(dir, 'workspace-restore-required.json')}`);
    }
  }
  if (browser) await browser.close();
  if (failedChecks) process.exitCode = 1;
} else {
  console.log('usage: eval-drive.mjs setup | run --model <substr> --tests <csv|all>');
}
