/**
 * Live MiniCPM fixture families. Skipped unless LOCALLM_LIVE_MINICPM=1.
 * Drives the shipped PDF extractor and the managed llama-server binary.
 */
import { execFileSync, spawn, type ChildProcess } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { zlibSync } from 'fflate';
import { afterAll, beforeAll, expect, it } from 'vitest';
import { extractPdfText } from '../documents/document_store';

const live = process.env.LOCALLM_LIVE_MINICPM === '1';
const root = resolve('.');
const packagedServer = join(process.env.APPDATA || '', 'app.locallm.desktop/runtimes/llama-b10855-cuda12.4-89ecea3d-f0f2-4389-b40c-e51060da4cd0/llama-server.exe');
const serverBin = process.env.LOCALLM_LLAMA_SERVER || packagedServer;
const model = process.env.LOCALLM_MODEL || join(root, '.local/models/MiniCPM5-2B.Q6_K.gguf');
const port = 8107;
const base = `http://127.0.0.1:${port}`;
const scratch = process.env.LOCALLM_LIVE_SCRATCH || '';

type Row = { pass: number; task: string; sameAsOtherPass: boolean | null; detail: string };
const rows: Row[] = [];
const recordedRequests: string[] = [];
const external: string[] = [];
let server: ChildProcess | null = null;
let serverLog = '';
const originalFetch = globalThis.fetch;

function utf8(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

function concat(parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

function compressedTariffPdf(): Uint8Array {
  const payload = zlibSync(utf8('(The tariff is 19.99) Tj'));
  return concat([
    utf8('%PDF-1.4\n1 0 obj << /Type /Page /Length 16 >> stream\n(Cover only) Tj\nendstream endobj\n'),
    utf8(`2 0 obj << /Type /Page /Filter /FlateDecode /Length ${payload.length} >> stream\n`),
    payload,
    utf8('\nendstream endobj\n'),
  ]);
}

async function ready(): Promise<void> {
  const deadline = Date.now() + 180000;
  while (Date.now() < deadline) {
    if (server?.exitCode !== null) throw new Error(`llama-server exited ${server?.exitCode}: ${serverLog.slice(-1500)}`);
    try {
      const res = await fetch(`${base}/health`);
      if (res.ok) return;
    } catch { /* still starting */ }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`llama-server did not become ready. ${serverLog.slice(-1500)}`);
}

async function complete(messages: unknown[], extra: Record<string, unknown> = {}): Promise<{ text: string; toolCalls: { name: string }[]; message: unknown; finalUrl: string; ms: number }> {
  const started = Date.now();
  const url = `${base}/v1/chat/completions`;
  let lastError: unknown;
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      const res = await fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          messages,
          temperature: 0,
          max_tokens: 128,
          stream: false,
          chat_template_kwargs: { enable_thinking: false },
          ...extra,
        }),
        signal: AbortSignal.timeout(120000),
      });
      const body = await res.text();
      if (!res.ok) throw new Error(`chat ${res.status}: ${body.slice(0, 300)}`);
      const json = JSON.parse(body);
      const message = json.choices?.[0]?.message ?? {};
      const toolCalls = (message.tool_calls ?? []).map((call: { function?: { name?: string } }) => ({ name: call.function?.name ?? '' }));
      return { text: String(message.content ?? ''), toolCalls, message, finalUrl: res.url || url, ms: Date.now() - started };
    } catch (error) {
      lastError = error;
      await new Promise((resolve) => setTimeout(resolve, 400));
    }
  }
  throw lastError;
}

const runCodeTool = {
  type: 'function',
  function: {
    name: 'run_code',
    description: 'Run Python. The last expression is the result.',
    parameters: { type: 'object', properties: { code: { type: 'string' } }, required: ['code'] },
  },
};

function shippedLedger(pass: number, message: unknown): {
  calculated: string; file: string; done: string; inflight: string; replayed: number;
  checkpoint: string; retrievalAllowed: boolean; remoteError: string; modelInvoked: number;
  modelCode: string; constraints: number;
} {
  const ws = join(scratch, `ws-${pass}`);
  mkdirSync(ws, { recursive: true });
  const messagePath = join(scratch, `model-message-${pass}.json`);
  const ledgerPath = join(scratch, `ledger-${pass}.json`);
  writeFileSync(messagePath, JSON.stringify(message));
  execFileSync(
    'cargo',
    ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--lib', 'shipped_packaged_entry', '--', '--nocapture'],
    {
      encoding: 'utf8',
      timeout: 180000,
      env: { ...process.env, LOCALLM_MODEL_MESSAGE: messagePath, LOCALLM_WORKSPACE: ws, LOCALLM_LEDGER_OUT: ledgerPath },
    },
  );
  return JSON.parse(readFileSync(ledgerPath, 'utf8'));
}

async function abortThenPong(): Promise<{ aborted: boolean; later: string; firstChunkBytes: number }> {
  const controller = new AbortController();
  const res = await fetch(`${base}/v1/chat/completions`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      messages: [{ role: 'user', content: 'Write a long story about a river. Keep going until you are stopped.' }],
      temperature: 0,
      max_tokens: 256,
      stream: true,
      chat_template_kwargs: { enable_thinking: false },
    }),
    signal: controller.signal,
  });
  if (!res.ok || !res.body) throw new Error(`stream ${res.status}`);
  const reader = res.body.getReader();
  const first = await reader.read();
  if (first.done || !first.value?.byteLength) throw new Error('stream ended before the first chunk');
  controller.abort();
  let aborted = false;
  try {
    await reader.read();
  } catch {
    aborted = true;
  }
  try { await reader.cancel(); } catch { /* the aborted read already closed the body */ }
  await new Promise((resolve) => setTimeout(resolve, 500));
  const later = await complete([{ role: 'user', content: 'Reply with exactly PONG and no other words.' }]);
  return { aborted, later: later.text, firstChunkBytes: first.value.byteLength };
}

beforeAll(async () => {
  if (!live) return;
  // Observe this JS process's fetch calls only. Native children and other
  // transports are outside this recorder; it cannot prove network isolation.
  globalThis.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = input instanceof Request ? input.url : String(input);
    recordedRequests.push(url);
    if (!['127.0.0.1', 'localhost', '[::1]'].includes(new URL(url).hostname)) external.push(url);
    return originalFetch(input, init);
  };
  server = spawn(serverBin, [
    '-m', model, '--jinja', '-c', '2048', '-ngl', '99',
    '--host', '127.0.0.1', '--port', String(port), '--no-webui',
  ], { stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
  server.stdout?.on('data', (chunk) => { serverLog += chunk; });
  server.stderr?.on('data', (chunk) => { serverLog += chunk; });
  await ready();
}, 200000);

afterAll(async () => {
  server?.kill();
  globalThis.fetch = originalFetch;
  if (!live || !scratch) return;
  const lines = [
    'direct managed-model fixtures and seeded Rust contracts on MiniCPM5-2B.Q6_K',
    `runtime=${serverBin}`,
    `model=${model}`,
    ...rows.map((row) => `pass=${row.pass} task=${row.task} same=${row.sameAsOtherPass} ${row.detail}`),
    `recorded_js_fetch_requests=${recordedRequests.length}`,
    `unexpected_js_external_requests=${external.length}`,
    'recorder_scope=JS fetch only; native child traffic unobserved; not network-isolation evidence',
    external.length ? `external=${external.join(' ')}` : 'external=none',
  ];
  writeFileSync(join(scratch, 'real-model-suite.log'), lines.join('\n') + '\n');
});

it.skipIf(!live)('runs direct MiniCPM fixtures and seeded Rust contracts twice', async () => {
  const pdfBytes = compressedTariffPdf();
  const extracted = extractPdfText(pdfBytes);
  expect(extracted.status).toBe('text_extracted');
  expect(extracted.needsOcr).toBe(false);
  expect(extracted.pages[1] ?? '').toContain('19.99');
  expect(extracted.pages[0] ?? '').not.toContain('19.99');

  const passes: Record<string, string>[] = [];
  for (const pass of [1, 2]) {
    const record: Record<string, string> = {};

    const explanation = await complete([{
      role: 'user',
      content: 'In two sentences, explain what a unit test is. Do not use tools.',
    }]);
    record['ordinary-completion'] = `ms=${explanation.ms} model=${JSON.stringify(explanation.text)}`;

    const pdfAnswer = await complete([{
      role: 'user',
      content: `Use only this extracted PDF text.\nPage 1: ${extracted.pages[0]}\nPage 2: ${extracted.pages[1]}\nWhat tariff number is stated, and on which page?`,
    }]);
    const pdfHit = /19\.99/.test(pdfAnswer.text) && /page\s*2|second page|\b2\b/i.test(pdfAnswer.text);
    record['supplied-pdf-evidence'] = `ms=${pdfAnswer.ms} extract_page2=${extracted.pages[1]} model=${JSON.stringify(pdfAnswer.text)} hit=${pdfHit}`;

    const comparison = await complete([{
      role: 'user',
      content: 'Source A says Alpha costs 10. Source B says Beta costs 12. Compare Alpha and Beta using both sources. Mention both names.',
    }]);
    const compared = /alpha/i.test(comparison.text) && /beta/i.test(comparison.text);
    record['supplied-source-comparison'] = `ms=${comparison.ms} both=${compared} model=${JSON.stringify(comparison.text)}`;

    const calc = await complete([{
      role: 'user',
      content: 'Call the run_code tool. Set its code argument to exactly round(19.99 * 1.08, 2). Do not calculate the number yourself.',
    }], { tools: [runCodeTool], tool_choice: { type: 'function', function: { name: 'run_code' } } });
    const ledger = shippedLedger(pass, calc.message);
    const savedOnce = String(ledger.file).includes('21.59') && ledger.calculated === '21.59' && String(ledger.file).split('21.59').length - 1 === 1;
    record['generated-calc-tool-handoff'] = `model=${JSON.stringify(calc.message)} calculated=${ledger.calculated} model_code=${JSON.stringify(ledger.modelCode)} model_message_parsed=${ledger.modelInvoked} saved=${JSON.stringify(ledger.file)} once=${savedOnce} replayed=${ledger.replayed} inflight=${ledger.inflight}`;

    const donePath = join(scratch, `ws-${pass}`, 'done.txt');
    const fileBefore = readFileSync(donePath, 'utf8').trim();
    const resume = await complete([{
      role: 'user',
      content: `${ledger.checkpoint}\nReply with exactly RESUMED and no other words.`,
    }]);
    const fileAfter = readFileSync(donePath, 'utf8').trim();
    const repeated = fileBefore !== fileAfter;
    record['seeded-checkpoint-recovery'] = `file_before=${fileBefore} file_after=${fileAfter} repeated=${repeated} resume=${JSON.stringify(resume.text)} checkpoint=${JSON.stringify(ledger.checkpoint)} inflight=${ledger.inflight} replayed=${ledger.replayed}`;

    const cancelled = await abortThenPong();
    record['http-stream-abort'] = `first_chunk=${cancelled.firstChunkBytes} aborted=${cancelled.aborted} later=${JSON.stringify(cancelled.later)}`;

    const externalBefore = external.length;
    const weather = await complete([{
      role: 'user',
      content: 'Search for the current weather in Paris.',
    }], {
      tools: [{ type: 'function', function: { name: 'web_search', description: 'Search the public web.', parameters: { type: 'object', properties: { query: { type: 'string' } }, required: ['query'] } } }],
      tool_choice: { type: 'function', function: { name: 'web_search' } },
    });
    const toolName = weather.toolCalls[0]?.name ?? 'none';
    record['generated-weather-tool-call'] = `js_external_delta=${external.length - externalBefore} tool=${toolName} call=${JSON.stringify(weather.message)}`;

    const remoteBefore = external.length;
    const local = await complete([{ role: 'user', content: 'What is 1+1? Reply with only the digit.' }]);
    const openaiRequests = external.slice(remoteBefore).filter((url) => /openai\.com/i.test(url)).length;
    record['local-loopback-completion'] = `text=${JSON.stringify(local.text)} final_url=${local.finalUrl} js_openai_requests=${openaiRequests}`;
    record['seeded-constraints-persistence'] = `constraints=${ledger.constraints}`;

    passes.push(record);
  }

  for (const task of Object.keys(passes[0])) {
    const outcome = (detail: string) => detail.replace(/(?:resume_)?ms=\d+/g, 'ms').replace(/\(\d+ms\)/g, '');
    const same = outcome(passes[0][task]) === outcome(passes[1][task]);
    rows.push({ pass: 1, task, sameAsOtherPass: same, detail: passes[0][task] });
    rows.push({ pass: 2, task, sameAsOtherPass: same, detail: passes[1][task] });
  }
  expect(recordedRequests).toContain(`${base}/health`);
  expect(recordedRequests.filter(url => url === `${base}/v1/chat/completions`).length).toBeGreaterThanOrEqual(18);
  expect(external).toEqual([]);
  for (const pass of passes) {
    expect(pass['supplied-pdf-evidence']).toContain('hit=true');
    expect(pass['supplied-source-comparison']).toContain('both=true');
    expect(pass['ordinary-completion'].length).toBeGreaterThan(40);
    expect(pass['generated-calc-tool-handoff']).toContain('calculated=21.59');
    expect(pass['generated-calc-tool-handoff']).toContain('model_message_parsed=1');
    expect(pass['generated-calc-tool-handoff']).toContain('once=true');
    expect(pass['generated-calc-tool-handoff']).toContain('replayed=0');
    expect(pass['generated-calc-tool-handoff']).toContain('inflight=unknown');
    expect(pass['generated-calc-tool-handoff']).toContain('run_code');
    expect(pass['seeded-checkpoint-recovery']).toContain('file_before=DONE');
    expect(pass['seeded-checkpoint-recovery']).toContain('file_after=DONE');
    expect(pass['seeded-checkpoint-recovery']).toContain('RESUMED');
    expect(pass['seeded-checkpoint-recovery']).toContain('Paused checkpoint');
    expect(pass['seeded-checkpoint-recovery']).toContain('inflight=unknown');
    expect(pass['seeded-checkpoint-recovery']).toContain('repeated=false');
    expect(pass['http-stream-abort']).toContain('aborted=true');
    expect(pass['http-stream-abort']).toMatch(/first_chunk=[1-9]/);
    expect(pass['http-stream-abort']).toContain('PONG');
    expect(pass['generated-weather-tool-call']).toContain('tool=web_search');
    expect(pass['generated-weather-tool-call']).toContain('js_external_delta=0');
    expect(pass['local-loopback-completion']).toContain('127.0.0.1:8107');
    expect(pass['local-loopback-completion']).toContain('js_openai_requests=0');
    expect(pass['local-loopback-completion']).toContain('"2"');
    expect(pass['seeded-constraints-persistence']).not.toContain('constraints=0');
  }
}, 420000);
