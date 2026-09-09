import { chromium, expect } from '@playwright/test';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// Local MCP chat acceptance: real MiniCPM -> local fixture tool -> approval ->
// tool result -> completed response -> persisted records verified after reload.
const directory = mkdtempSync(join(tmpdir(), 'locallm-mcp-chat-'));
const name = `Local chat fixture ${Date.now()}`;
const script = join(directory, 'server.cjs');
writeFileSync(script, `
require('node:readline').createInterface({input:process.stdin}).on('line', line => {
  let r; try { r = JSON.parse(line); } catch { return; }
  if (r.id === undefined) return;
  let result;
  if (r.method === 'initialize') {
    result = {protocolVersion:r.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'chat-fixture',version:'1'}};
  } else if (r.method === 'tools/list') {
    result = {tools:[
      {name:'read_file',description:'Read a fixture value',inputSchema:{type:'object',properties:{path:{type:'string'}},required:['path'],additionalProperties:false}},
      {name:'mutate_record',description:'Change external fixture state',inputSchema:{type:'object',properties:{value:{type:'string'}},required:['value'],additionalProperties:false}}
    ]};
  } else if (r.method === 'tools/call' && r.params?.name === 'read_file') {
    result = {content:[{type:'text',text:'FIXTURE-VALUE-4721'}],isError:false};
  } else if (r.method === 'tools/call') {
    result = {content:[{type:'text',text:'mutated'}],isError:false};
  } else {
    result = {tools:[]};
  }
  process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,result})+'\\n');
});`);
const callsSeen = join(directory, 'calls.json');
writeFileSync(callsSeen, '[]');

const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(() => { page = browser.contexts()[0].pages().find(value => value.url().includes('1420')); return Boolean(page); }).toBe(true);
page.setDefaultTimeout(15000);
const invoke = (command, args = {}) => page.evaluate(async ({ command, args }) => {
  const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
  return invoke(command, args);
}, { command, args });
const report = { testedAt: new Date().toISOString(), name, scenarios: [] };
let id;
let chatId;
let originalPreferences;
let originalActiveSkills = [];
try {
  // Managed installs were removed since the last acceptance run and only
  // ~1.5 GiB is free, so point the app at the existing developer assets
  // (2.07 GB model + extracted runtime) instead of re-downloading.
  const prefs = (await invoke('bootstrap')).preferences;
  if (!prefs.runtimePath || !prefs.modelPath) {
    const runtime = join(process.cwd(), '.local', 'runtime', 'llama-server.exe');
    const model = join(process.cwd(), '.local', 'models', 'MiniCPM5-2B.Q6_K.gguf');
    await invoke('save_preferences', { preferences: { ...prefs, runtimePath: runtime, modelPath: model, temperature: 0, maxTokens: 2048 } });
  } else {
    await invoke('save_preferences', { preferences: { ...prefs, temperature: 0, maxTokens: 2048 } });
  }
  originalPreferences = (await invoke('bootstrap')).preferences;
  originalActiveSkills = (await invoke('list_skills')).filter(item => item.active).map(item => item.id);
  for (const skill of originalActiveSkills) await invoke('set_skill_active', { id: skill, active: false });
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Load model', exact: true })).toBeEnabled({ timeout: 30000 });
  if (await page.getByRole('button', { name: 'Load model', exact: true }).count()) {
    await page.getByRole('button', { name: 'Load model', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Unload', exact: true })).toBeVisible({ timeout: 180000 });
  }
  await invoke('save_local_connector', {
    server: {
      id: `local-${Date.now().toString(16)}-0000-4000-8000-000000000000`.slice(0, 44),
      name, executable: process.execPath, arguments: [script], workingDirectory: directory, environment: {},
    },
  }).catch(async () => {
    // Fall back to UI save when direct IPC validation rejects the generated ID shape.
    await page.getByRole('button', { name: 'Connectors', exact: true }).click();
    await page.getByText('Add a local MCP server', { exact: true }).click();
    await page.getByLabel('Name', { exact: true }).fill(name);
    await page.getByLabel('Executable path', { exact: true }).fill(process.execPath);
    await page.getByLabel('Working directory', { exact: true }).fill(directory);
    await page.getByLabel('Arguments (JSON array)', { exact: true }).fill(JSON.stringify([script]));
    await page.getByLabel('Environment variables (JSON object)', { exact: true }).fill('{}');
    await page.getByRole('button', { name: 'Save local server', exact: true }).click();
    await expect(page.getByRole('status')).toContainText('Saved.');
  });
  const saved = (await invoke('list_connectors')).find(item => item.description === name);
  if (!saved) throw new Error('Local fixture was not saved.');
  id = saved.id;
  await invoke('connect_connector', { id });
  expect((await invoke('list_connectors')).find(item => item.id === id).tools.map(tool => tool.name).sort()).toEqual(['mutate_record', 'read_file']);

  async function runTurn({ mode, tools, prompt, decision, promptCheck }) {
    const chat = await invoke('create_conversation');
    const current = chat.id;
    try {
      await invoke('save_conversation_tools', { id: current, tools: { sources: [], tools, accessMode: mode } });
      await invoke('save_preferences', { preferences: { ...(await invoke('bootstrap')).preferences, temperature: 0, maxTokens: 2048 } });
      const { Channel } = await import('/node_modules/@tauri-apps/api/core.js').catch(() => ({}));
      const outcome = await page.evaluate(async ({ chatId, prompt, tools, decision }) => {
        const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
        const channel = new Channel();
        const approvals = [];
        channel.onmessage = event => {
          if (event.approval) {
            approvals.push({ id: event.approval.id, connector: event.approval.connector, localServerName: event.approval.localServerName, name: event.approval.name, args: event.approval.arguments });
            approvals.push(invoke('resolve_tool_approval', { id: event.approval.id, allow: decision === 'allow' }));
          }
        };
        await invoke('send_message', { conversationId: chatId, content: prompt, connectorIds: [], connectorTools: tools, channel });
        await Promise.all(approvals.slice(1));
        return approvals[0] && typeof approvals[0] === 'object' ? approvals[0] : null;
      }, { chatId: current, prompt, tools, decision });
      void Channel;
      void promptCheck;
      return { chat: current, approval: outcome };
    } catch (error) {
      await invoke('delete_conversation', { id: current }).catch(() => {});
      throw error;
    }
  }

  // 1. Ask + allow: identity, arguments, result continuation, durable record.
  {
    const tools = [{ connectorId: id, toolName: 'read_file' }];
    const prompt = `Use read_file exactly once with {"path":"fixture.txt"}. Then reply with the exact returned value FIXTURE-VALUE-4721. Do not call any other tool.`;
    const { chat, approval } = await runTurn({ mode: 'ask', tools, prompt, decision: 'allow' });
    chatId = chat;
    const messages = await invoke('get_messages', { id: chat });
    const audits = messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
    const answer = messages.find(message => message.role === 'assistant')?.content ?? '';
    report.scenarios.push({
      case: 'ask-allow',
      approvalConnector: approval?.connector, approvalServerName: approval?.localServerName,
      approvalTool: approval?.name, approvalArgs: approval?.args,
      decision: audits[0]?.request?.decision, accessMode: audits[0]?.request?.accessMode,
      authorization: audits[0]?.request?.authorization,
      resultText: JSON.stringify(audits[0]?.result), answerIncludesValue: answer.includes('FIXTURE-VALUE-4721'),
      auditCount: audits.length, assistantStatus: messages.find(message => message.role === 'assistant')?.status,
    });
  }

  // 2. Deny: no result execution, no retry later in the turn.
  {
    const tools = [{ connectorId: id, toolName: 'mutate_record' }];
    const prompt = `Use mutate_record exactly once with {"value":"should-not-happen"}. If denied, stop and say denied. Do not call any other tool.`;
    const chat = await invoke('create_conversation');
    try {
      await invoke('save_conversation_tools', { id: chat.id, tools: { sources: [], tools, accessMode: 'ask' } });
      const approval = await page.evaluate(async ({ chatId, prompt, tools }) => {
        const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
        const channel = new Channel();
        let seen = null;
        const pending = [];
        channel.onmessage = event => {
          if (event.approval && !seen) {
            seen = { id: event.approval.id, connector: event.approval.connector, localServerName: event.approval.localServerName, name: event.approval.name, args: event.approval.arguments };
            pending.push(invoke('resolve_tool_approval', { id: event.approval.id, allow: false }));
          } else if (event.approval) {
            pending.push(invoke('resolve_tool_approval', { id: event.approval.id, allow: false }));
          }
        };
        await invoke('send_message', { conversationId: chatId, content: prompt, connectorIds: [], connectorTools: tools, channel });
        await Promise.all(pending);
        return seen;
      }, { chatId: chat.id, prompt, tools });
      const messages = await invoke('get_messages', { id: chat.id });
      const audits = messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
      const answer = messages.find(message => message.role === 'assistant')?.content ?? '';
      report.scenarios.push({
        case: 'ask-deny', approvalConnector: approval?.connector, approvalServerName: approval?.localServerName,
        approvalTool: approval?.name, decision: audits[0]?.request?.decision,
        auditCount: audits.length, deniedMentions: answer.toLowerCase().includes('denied'),
      });
    } finally { await invoke('delete_conversation', { id: chat.id }).catch(() => {}); }
  }

  // 3. Auto-approve reads still prompts for a local tool named read_file.
  {
    const tools = [{ connectorId: id, toolName: 'read_file' }];
    const prompt = `Use read_file exactly once with {"path":"fixture.txt"}. If denied, stop and say denied.`;
    const chat = await invoke('create_conversation');
    try {
      await invoke('save_conversation_tools', { id: chat.id, tools: { sources: [], tools, accessMode: 'autoApprove' } });
      const outcome = await page.evaluate(async ({ chatId, prompt, tools }) => {
        const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
        const channel = new Channel();
        let prompts = 0;
        const pending = [];
        channel.onmessage = event => {
          if (event.approval) { prompts++; pending.push(invoke('resolve_tool_approval', { id: event.approval.id, allow: false })); }
        };
        await invoke('send_message', { conversationId: chatId, content: prompt, connectorIds: [], connectorTools: tools, channel });
        await Promise.all(pending);
        return { prompts };
      }, { chatId: chat.id, prompt, tools });
      const messages = await invoke('get_messages', { id: chat.id });
      const audits = messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
      report.scenarios.push({ case: 'autoApprove-local-read_file', prompts: outcome.prompts, decision: audits[0]?.request?.decision, auditCount: audits.length });
    } finally { await invoke('delete_conversation', { id: chat.id }).catch(() => {}); }
  }

  // 4. Full access executes without a prompt.
  {
    const tools = [{ connectorId: id, toolName: 'read_file' }];
    const prompt = `Use read_file exactly once with {"path":"fixture.txt"}. Then reply with the exact returned value FIXTURE-VALUE-4721.`;
    const chat = await invoke('create_conversation');
    try {
      await invoke('save_conversation_tools', { id: chat.id, tools: { sources: [], tools, accessMode: 'fullAccess' } });
      const outcome = await page.evaluate(async ({ chatId, prompt, tools }) => {
        const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
        const channel = new Channel();
        let prompts = 0;
        channel.onmessage = event => { if (event.approval) prompts++; };
        await invoke('send_message', { conversationId: chatId, content: prompt, connectorIds: [], connectorTools: tools, channel });
        return { prompts };
      }, { chatId: chat.id, prompt, tools });
      const messages = await invoke('get_messages', { id: chat.id });
      const audits = messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
      const answer = messages.find(message => message.role === 'assistant')?.content ?? '';
      report.scenarios.push({
        case: 'fullAccess-no-prompt', prompts: outcome.prompts,
        decision: audits[0]?.request?.decision, authorization: audits[0]?.request?.authorization,
        answerIncludesValue: answer.includes('FIXTURE-VALUE-4721'), auditCount: audits.length,
      });
    } finally { await invoke('delete_conversation', { id: chat.id }).catch(() => {}); }
  }

  // 5. Saved records survive reload with identity, arguments, authorization, results.
  await page.reload();
  await expect.poll(() => { const next = browser.contexts()[0].pages().find(value => value.url().includes('1420')); if (next) page = next; return Boolean(page); }).toBe(true);
  {
    const messages = await invoke('get_messages', { id: chatId });
    const audits = messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
    report.scenarios.push({
      case: 'reload-persistence',
      request: audits[0]?.request, resultText: JSON.stringify(audits[0]?.result),
      assistantStatus: messages.find(message => message.role === 'assistant')?.status,
      messageCount: messages.length,
    });
  }

  writeFileSync('test-results/local-mcp-chat-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));

  expect(report.scenarios[0].approvalConnector).toBe(id);
  expect(report.scenarios[0].approvalServerName).toBe(name);
  expect(report.scenarios[0].approvalTool).toBe('read_file');
  expect(report.scenarios[0].approvalArgs).toEqual({ path: 'fixture.txt' });
  expect(report.scenarios[0].decision).toBe('allowed');
  expect(report.scenarios[0].accessMode).toBe('ask');
  expect(report.scenarios[0].answerIncludesValue).toBe(true);
  expect(report.scenarios[0].assistantStatus).toBe('complete');
  expect(report.scenarios[1].decision).toBe('denied');
  expect(report.scenarios[1].auditCount).toBe(1);
  expect(report.scenarios[1].deniedMentions).toBe(true);
  expect(report.scenarios[2].prompts).toBe(1);
  expect(report.scenarios[2].decision).toBe('denied');
  expect(report.scenarios[3].prompts).toBe(0);
  expect(report.scenarios[3].decision).toBe('allowed');
  expect(report.scenarios[3].authorization).toBe('full access selected by user');
  expect(report.scenarios[3].answerIncludesValue).toBe(true);
  expect(report.scenarios[4].request.connector).toBe(id);
  expect(report.scenarios[4].request.localServerName).toBe(name);
  expect(report.scenarios[4].request.decision).toBe('allowed');
  expect(report.scenarios[4].resultText).toContain('FIXTURE-VALUE-4721');
} finally {
  try {
    if (chatId) await invoke('delete_conversation', { id: chatId }).catch(() => {});
    if (id && (await invoke('list_local_connectors')).some(item => item.id === id)) {
      await invoke('disconnect_connector', { id, forget: false }).catch(() => {});
      await invoke('remove_local_connector', { id }).catch(() => {});
    }
    if (originalPreferences) await invoke('save_preferences', { preferences: originalPreferences }).catch(() => {});
    for (const skill of originalActiveSkills) await invoke('set_skill_active', { id: skill, active: true }).catch(() => {});
  } catch { /* best-effort restore */ }
  await browser.close().catch(() => {});
  rmSync(directory, { recursive: true, force: true });
}
