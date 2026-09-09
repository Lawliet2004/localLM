import { chromium, expect } from '@playwright/test';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';

// Local MCP failure-mode acceptance: cancellation during approval, tool
// failure, unexpected server exit, and disconnect each produce accurate
// durable outcomes with the real MiniCPM model.
const directory = mkdtempSync(join(tmpdir(), 'locallm-mcp-fail-'));
const name = `Local failure fixture ${Date.now()}`;
const script = join(directory, 'server.cjs');
writeFileSync(script, `
require('node:readline').createInterface({input:process.stdin}).on('line', line => {
  let r; try { r = JSON.parse(line); } catch { return; }
  if (r.id === undefined) return;
  if (r.method === 'initialize') {
    process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,result:{protocolVersion:r.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'fail-fixture',version:'1'}}})+'\\n');
  } else if (r.method === 'tools/list') {
    process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,result:{tools:[
      {name:'slow_echo',description:'Echo back slowly',inputSchema:{type:'object',properties:{message:{type:'string'}},required:['message'],additionalProperties:false}},
      {name:'fail_tool',description:'Always fail',inputSchema:{type:'object',properties:{},additionalProperties:false}},
      {name:'die_tool',description:'Exit before answering',inputSchema:{type:'object',properties:{},additionalProperties:false}}
    ]}})+'\\n');
  } else if (r.method === 'tools/call' && r.params?.name === 'slow_echo') {
    process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,result:{content:[{type:'text',text:'slow:'+(r.params.arguments?.message ?? '')}],isError:false}})+'\\n');
  } else if (r.method === 'tools/call' && r.params?.name === 'fail_tool') {
    process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,result:{content:[{type:'text',text:'fixture failure detail'}],isError:true}})+'\\n');
  } else if (r.method === 'tools/call') {
    process.exit(1);
  }
});`);

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
const chats = [];
let originalPreferences;
try {
  originalPreferences = (await invoke('bootstrap')).preferences;
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  if (await page.getByRole('button', { name: 'Load model', exact: true }).count()) {
    await expect(page.getByRole('button', { name: 'Load model', exact: true })).toBeEnabled({ timeout: 30000 });
    await page.getByRole('button', { name: 'Load model', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Unload', exact: true })).toBeVisible({ timeout: 180000 });
  }
  await invoke('save_preferences', { preferences: { ...(await invoke('bootstrap')).preferences, temperature: 0, maxTokens: 2048 } });
  id = `local-${randomUUID()}`;
  await invoke('save_local_connector', {
    server: { id, name, executable: process.execPath, arguments: [script], workingDirectory: directory, environment: {} },
  });
  await invoke('connect_connector', { id });
  expect((await invoke('list_connectors')).find(item => item.id === id).tools.map(tool => tool.name).sort())
    .toEqual(['die_tool', 'fail_tool', 'slow_echo']);

  // 1. Cancellation during approval: interrupted, zero tool records.
  {
    const chat = await invoke('create_conversation');
    chats.push(chat.id);
    const tools = [{ connectorId: id, toolName: 'slow_echo' }];
    await invoke('save_conversation_tools', { id: chat.id, tools: { sources: [], tools, accessMode: 'ask' } });
    const approval = await page.evaluate(async ({ chatId, tools }) => {
      const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
      const channel = new Channel();
      let seen = null;
      channel.onmessage = async event => {
        if (event.approval && !seen) {
          seen = { name: event.approval.name, server: event.approval.localServerName, args: event.approval.arguments };
          await invoke('cancel_generation');
        }
      };
      await invoke('send_message', { conversationId: chatId, content: 'Use slow_echo exactly once with {"message":"hello"}.', connectorIds: [], connectorTools: tools, channel });
      return seen;
    }, { chatId: chat.id, tools });
    const messages = await invoke('get_messages', { id: chat.id });
    const audits = messages.filter(message => message.role === 'tool');
    const assistant = messages.find(message => message.role === 'assistant');
    report.scenarios.push({
      case: 'cancel-during-approval', approvalTool: approval?.name, approvalServer: approval?.server,
      approvalArgs: approval?.args, assistantStatus: assistant?.status, toolRecords: audits.length,
      messageRoles: messages.map(message => `${message.role}:${message.status}`),
    });
  }

  // 2. Tool failure: durable error result, completed response.
  {
    const chat = await invoke('create_conversation');
    chats.push(chat.id);
    const tools = [{ connectorId: id, toolName: 'fail_tool' }];
    await invoke('save_conversation_tools', { id: chat.id, tools: { sources: [], tools, accessMode: 'ask' } });
    await page.evaluate(async ({ chatId, tools }) => {
      const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
      const channel = new Channel();
      const pending = [];
      channel.onmessage = event => {
        if (event.approval) pending.push(invoke('resolve_tool_approval', { id: event.approval.id, allow: true }));
      };
      await invoke('send_message', { conversationId: chatId, content: 'Use fail_tool exactly once with {}. If it fails, briefly say it failed.', connectorIds: [], connectorTools: tools, channel });
      await Promise.all(pending);
    }, { chatId: chat.id, tools });
    const messages = await invoke('get_messages', { id: chat.id });
    const audits = messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
    const assistant = messages.find(message => message.role === 'assistant');
    report.scenarios.push({
      case: 'tool-failure', decision: audits[0]?.request?.decision,
      resultIsError: audits[0]?.result?.isError,
      resultText: JSON.stringify(audits[0]?.result),
      assistantStatus: assistant?.status, answerMentionsFail: (assistant?.content ?? '').toLowerCase().includes('fail'),
    });
  }

  // 3. Unexpected server exit mid-call: unknown-outcome error, no retry.
  {
    const chat = await invoke('create_conversation');
    chats.push(chat.id);
    const tools = [{ connectorId: id, toolName: 'die_tool' }];
    await invoke('save_conversation_tools', { id: chat.id, tools: { sources: [], tools, accessMode: 'ask' } });
    await page.evaluate(async ({ chatId, tools }) => {
      const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
      const channel = new Channel();
      const pending = [];
      channel.onmessage = event => {
        if (event.approval) pending.push(invoke('resolve_tool_approval', { id: event.approval.id, allow: true }));
      };
      await invoke('send_message', { conversationId: chatId, content: 'Use die_tool exactly once with {}. Then briefly describe the outcome.', connectorIds: [], connectorTools: tools, channel });
      await Promise.all(pending);
    }, { chatId: chat.id, tools });
    const messages = await invoke('get_messages', { id: chat.id });
    const audits = messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content));
    const assistant = messages.find(message => message.role === 'assistant');
    report.scenarios.push({
      case: 'server-exit', decision: audits[0]?.request?.decision,
      resultIsError: audits[0]?.result?.isError,
      resultText: JSON.stringify(audits[0]?.result),
      auditCount: audits.length, assistantStatus: assistant?.status,
    });
  }

  // 4. Disconnect before the turn: send is rejected, nothing is persisted.
  {
    await invoke('disconnect_connector', { id, forget: false });
    const chat = await invoke('create_conversation');
    chats.push(chat.id);
    const tools = [{ connectorId: id, toolName: 'slow_echo' }];
    await invoke('save_conversation_tools', { id: chat.id, tools: { sources: [], tools, accessMode: 'ask' } });
    const error = await page.evaluate(async ({ chatId, tools }) => {
      const { invoke, Channel } = await import('/node_modules/@tauri-apps/api/core.js');
      const channel = new Channel();
      channel.onmessage = () => {};
      try {
        await invoke('send_message', { conversationId: chatId, content: 'Use slow_echo once.', connectorIds: [], connectorTools: tools, channel });
        return null;
      } catch (e) { return String(e); }
    }, { chatId: chat.id, tools });
    const messages = await invoke('get_messages', { id: chat.id });
    report.scenarios.push({ case: 'disconnected-send', error, persistedMessages: messages.length });
  }

  // 5. Records survive a UI reload with identity, authorization, and results.
  await page.reload();
  await expect.poll(() => { page = browser.contexts()[0].pages().find(value => value.url().includes('1420')); return Boolean(page); }).toBe(true);
  {
    const reread = [];
    for (const chatId of chats.slice(0, 3)) {
      const messages = await invoke('get_messages', { id: chatId });
      reread.push(messages.filter(message => message.role === 'tool').map(message => JSON.parse(message.content)));
    }
    report.scenarios.push({
      case: 'reload-persistence',
      cancelRecords: reread[0].length,
      failureDecision: reread[1][0]?.request?.decision,
      failureIsError: reread[1][0]?.result?.isError,
      exitResult: JSON.stringify(reread[2][0]?.result),
      exitConnector: reread[2][0]?.request?.connector,
      exitServer: reread[2][0]?.request?.localServerName,
    });
  }

  writeFileSync('test-results/local-mcp-failure-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));

  expect(report.scenarios[0].approvalTool).toBe('slow_echo');
  expect(report.scenarios[0].approvalServer).toBe(name);
  expect(report.scenarios[0].approvalArgs).toEqual({ message: 'hello' });
  expect(report.scenarios[0].assistantStatus).toBe('interrupted');
  expect(report.scenarios[0].toolRecords).toBe(0);
  expect(report.scenarios[1].decision).toBe('allowed');
  expect(report.scenarios[1].resultIsError).toBe(true);
  expect(report.scenarios[1].resultText).toContain('fixture failure detail');
  expect(report.scenarios[1].assistantStatus).toBe('complete');
  expect(report.scenarios[2].decision).toBe('allowed');
  expect(report.scenarios[2].resultIsError).toBe(true);
  expect(report.scenarios[2].resultText).toContain('remote outcome may be unknown');
  expect(report.scenarios[2].auditCount).toBe(1);
  expect(report.scenarios[2].assistantStatus).toBe('complete');
  expect(report.scenarios[3].error).toContain('not connected');
  expect(report.scenarios[3].persistedMessages).toBe(0);
  expect(report.scenarios[4].cancelRecords).toBe(0);
  expect(report.scenarios[4].failureDecision).toBe('allowed');
  expect(report.scenarios[4].failureIsError).toBe(true);
  expect(report.scenarios[4].exitResult).toContain('remote outcome may be unknown');
  expect(report.scenarios[4].exitConnector).toBe(id);
  expect(report.scenarios[4].exitServer).toBe(name);
} finally {
  try {
    for (const chatId of chats) await invoke('delete_conversation', { id: chatId }).catch(() => {});
    if (id && (await invoke('list_local_connectors')).some(item => item.id === id)) {
      await invoke('disconnect_connector', { id, forget: false }).catch(() => {});
      await invoke('remove_local_connector', { id }).catch(() => {});
    }
    if (originalPreferences) await invoke('save_preferences', { preferences: originalPreferences }).catch(() => {});
  } catch { /* best-effort restore */ }
  await browser.close().catch(() => {});
  rmSync(directory, { recursive: true, force: true });
}
