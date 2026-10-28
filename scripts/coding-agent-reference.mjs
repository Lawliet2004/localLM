// Deterministic OpenCode behavior evidence via REA. No user workspace,
// credentials, agent registration, or external model provider is used.
import { createServer } from 'node:http';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdir, mkdtemp, writeFile, readFile } from 'node:fs/promises';
import { tmpdir, homedir } from 'node:os';
import { resolve, join } from 'node:path';
import { createHash } from 'node:crypto';
const exec = promisify(execFile);
const output = resolve(process.env.LOCALLM_REFERENCE_OUTPUT || '.local/qa/coding-agent-reference');
await mkdir(output, { recursive: true });
const isolated = await mkdtemp(join(tmpdir(), 'locallm-opencode-reference-'));
const workspace = join(isolated, 'workspace');
await mkdir(workspace);
await writeFile(join(workspace, 'value.txt'), 'broken\n');
await writeFile(join(workspace, 'check.sh'), `printf '1..1\\n'\nif [ "$(cat value.txt)" = fixed ]; then printf 'ok 1 - value\\n'; else printf 'not ok 1 - value\\n'; exit 1; fi\n`);
const binary = process.env.LOCALLM_OPENCODE_BIN || join(homedir(), '.opencode/bin/opencode');
const requests = [];
let stage = 0;
const server = createServer(async (req, res) => {
  try {
    let text = '';
    for await (const chunk of req) {
      text += chunk;
      if (text.length > 4 * 1024 * 1024) throw new Error('Fixture request too large');
    }
    const body = JSON.parse(text || '{}');
    const main = Array.isArray(body.tools) && body.tools.length > 0;
    const step = main ? stage++ : -1;
    const call = (name, args) => ({ index: 0, id: `fixture-${step}`, type: 'function', function: { name, arguments: JSON.stringify(args) } });
    const shellName = body.tools?.some(tool => tool.function?.name === 'shell') ? 'shell' : 'bash';
    const calls = [
      () => call('read', { path: join(workspace, 'value.txt') }),
      () => call('edit', { path: join(workspace, 'value.txt'), oldString: 'broken', newString: 'still-broken' }),
      () => call(shellName, { command: 'sh check.sh', ...(shellName === 'shell' ? { workdir: workspace, timeout: 10000 } : { description: 'Run the failing fixture check' }) }),
      () => call('read', { path: join(workspace, 'value.txt') }),
      () => call('edit', { path: join(workspace, 'value.txt'), oldString: 'still-broken', newString: 'fixed' }),
      () => call(shellName, { command: 'sh check.sh', ...(shellName === 'shell' ? { workdir: workspace, timeout: 10000 } : { description: 'Verify the repaired fixture' }) }),
    ];
    const tool = calls[step]?.();
    if (step === 0) await writeFile(join(output, 'advertised-tools.json'), JSON.stringify(body.tools, null, 2));
    requests.push({ path: req.url, step, toolNames: body.tools?.map(tool => tool.function?.name), toolSchemas: step === 0 ? body.tools : undefined, messages: body.messages });
    const delta = tool ? { role: 'assistant', content: 'I will inspect the result and continue.', tool_calls: [tool] } : { role: 'assistant', content: main ? 'Fixed value.txt. The first check failed; after repair, one test passed.' : 'Coding workflow fixture' };
    res.writeHead(200, { 'Content-Type': 'text/event-stream' });
    const event = (delta, finish_reason = null) => `data: ${JSON.stringify({ id: `completion-${step}`, object: 'chat.completion.chunk', created: 1, model: 'coding-fixture', choices: [{ index: 0, delta, finish_reason }] })}\n\n`;
    res.end(event(delta) + event({}, tool ? 'tool_calls' : 'stop') + 'data: [DONE]\n\n');
  } catch (error) { res.writeHead(500); res.end(String(error)); }
});
await new Promise((done, fail) => { server.once('error', fail); server.listen(0, '127.0.0.1', done); });
try {
  const config = join(isolated, 'opencode.json');
  await writeFile(config, JSON.stringify({ autoupdate: false, share: 'disabled', enabled_providers: ['fixture'], permission: 'allow', provider: { fixture: { npm: '@ai-sdk/openai-compatible', name: 'Local deterministic fixture', options: { baseURL: `http://127.0.0.1:${server.address().port}/v1`, apiKey: 'fixture-only' }, models: { 'coding-fixture': { name: 'Fixture', limit: { context: 65536, output: 4096 } } } } } }));
  // env -i prevents the captured application inheriting provider credentials.
  const scenario = { executable: '/usr/bin/env', arguments: ['-i', 'PATH=/usr/bin:/bin', 'LANG=C.UTF-8', `XDG_CONFIG_HOME=${isolated}/config`, `XDG_DATA_HOME=${isolated}/data`, `XDG_CACHE_HOME=${isolated}/cache`, `XDG_STATE_HOME=${isolated}/state`, `OPENCODE_CONFIG=${config}`, 'OPENCODE_DISABLE_CLAUDE_CODE=1', 'OPENCODE_DISABLE_AUTOUPDATE=1', binary, 'run', '--standalone', '--format', 'json', '--model', 'fixture/coding-fixture', 'Read value.txt, edit it, run check.sh, inspect its failure, repair the file, and rerun the check. Report the final result.'], working_directory: workspace, filesystem_observation_paths: [join(workspace, 'value.txt')], terminal: { columns: 120, rows: 40, scrollback: 2000 }, timeout_ms: 90000, idle_timeout_ms: 60000, settle_ms: 500 };
  const scenarioPath = join(output, 'opencode-scenario.json');
  await writeFile(scenarioPath, JSON.stringify(scenario, null, 2));
  const version = (await exec(binary, ['--version'])).stdout.trim();
  const provenance = { repository: 'https://github.com/NabilRFG12/Reverse-Engineer-anything', reaVersion: '6.7.0', opencodeVersion: version, binarySha256: createHash('sha256').update(await readFile(binary)).digest('hex'), node: process.version, isolatedWorkspace: workspace, startedAt: new Date().toISOString(), evidenceKind: 'scripted-provider runtime; not model-quality measurement' };
  await writeFile(join(output, 'provenance.json'), JSON.stringify(provenance, null, 2));
  const capture = await exec('npx', ['-y', 'rea-agents@6.7.0', 'capture-process', scenarioPath, '--format', 'json', '--full-output'], { timeout: 120000, maxBuffer: 16 * 1024 * 1024 });
  await writeFile(join(output, 'opencode-capture.json'), capture.stdout);
  await writeFile(join(output, 'rea-stderr.txt'), capture.stderr);
  await writeFile(join(output, 'provider-requests.json'), JSON.stringify(requests, null, 2));
  const value = await readFile(join(workspace, 'value.txt'), 'utf8');
  const evidence = JSON.parse(capture.stdout);
  if (!evidence.ok) throw new Error('REA capture failed: ' + JSON.stringify(evidence));
  const terminal = evidence.data.normalized_result.frames.map(frame => frame.data).join('');
  const events = terminal.split(/\r?\n/).flatMap(line => { try { return [JSON.parse(line)]; } catch { return []; } });
  const tools = events.filter(event => event.type === 'tool_use').map(event => event.part);
  const checks = tools.filter(tool => tool.tool === 'shell' || tool.tool === 'bash');
  const checkExitCodes = checks.map(tool => tool.state.metadata?.metadata?.exit);
  const result = { checkExitCodes, completed: stage === 7 && value === 'fixed\n' && checks.length === 2 && checkExitCodes[0] === 1 && checkExitCodes[1] === 0 && JSON.stringify(checks[0]).includes('not ok 1') && JSON.stringify(checks[1]).includes('ok 1') && !JSON.stringify(checks[1]).includes('not ok 1'), mainRequests: stage, observedTools: tools.map(tool => ({ tool: tool.tool, status: tool.state.status })), finalFile: value, capture: join(output, 'opencode-capture.json') };
  await writeFile(join(output, 'result.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
  if (!result.completed) process.exitCode = 1;
} finally { server.closeAllConnections(); await new Promise(done => server.close(done)); }
