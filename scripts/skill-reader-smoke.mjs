import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';

const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(() => { page = browser.contexts()[0].pages().find(item => item.url().includes('1420')); return Boolean(page); }).toBe(true);
page.setDefaultTimeout(15000);
async function invoke(command, args = {}) {
  return page.evaluate(async ({command, args}) => {
    const {invoke} = await import('/node_modules/@tauri-apps/api/core.js');
    return invoke(command, args);
  }, {command, args});
}
try {
  await page.getByRole('button', {name:'Models & runtime', exact:true}).click();
  if (await page.getByRole('button', {name:'Load model', exact:true}).count()) {
    await page.getByRole('button', {name:'Load model', exact:true}).click();
    await expect(page.getByRole('button', {name:'Unload', exact:true})).toBeVisible({timeout:120000});
  }
  const report = await page.evaluate(async () => {
    const {invoke, Channel} = await import('/node_modules/@tauri-apps/api/core.js');
    const preferences = (await invoke('bootstrap')).preferences;
    const active = (await invoke('list_skills')).filter(item => item.active).map(item => item.id);
    const results = [];
    try {
      for (const id of active) await invoke('set_skill_active', {id, active:false});
      await invoke('set_skill_active', {id:'jupyter-notebook', active:true});
      await invoke('save_preferences', {preferences:{...preferences, temperature:0, maxTokens:2048}});
      const expected = (await invoke('read_skill_file', {id:'jupyter-notebook',path:'references/quality-checklist.md'})).split(/\r?\n/).slice(0,3);
      for (const mode of ['ask', 'autoApprove', 'fullAccess']) {
        const chat = await invoke('create_conversation');
        try {
          await invoke('save_conversation_tools', {id:chat.id, tools:{sources:[],tools:[],accessMode:mode}});
          let prompts = 0;
          const decisions = [];
          const channel = new Channel();
          channel.onmessage = event => {
            if (event.approval) {
              prompts++;
              decisions.push(invoke('resolve_tool_approval', {id:event.approval.id,allow:true}));
            }
          };
          await invoke('send_message', {conversationId:chat.id,content:'Use skills_read_file exactly once with skill_id="jupyter-notebook", path="references/quality-checklist.md", start_line=1, line_count=3. Then quote those three lines. Do not create or execute anything.',connectorIds:[],connectorTools:[],channel});
          await Promise.all(decisions);
          const messages = await invoke('get_messages', {id:chat.id});
          results.push({mode,prompts,expected,audits:messages.filter(item => item.role === 'tool').map(item => JSON.parse(item.content)),answer:messages.find(item => item.role === 'assistant')?.content});
        } finally { await invoke('delete_conversation', {id:chat.id}); }
      }
      return results;
    } finally {
      await invoke('save_preferences', {preferences});
      await invoke('set_skill_active', {id:'jupyter-notebook',active:false});
      for (const id of active) await invoke('set_skill_active', {id,active:true});
    }
  });
  for (const item of report) {
    expect(item.prompts).toBe(item.mode === 'fullAccess' ? 0 : 1);
    expect(item.audits).toHaveLength(1);
    expect(item.audits[0].request.connector).toBe('Skills');
    expect(item.audits[0].result.lines.map(line => line.text)).toEqual(item.expected);
    expect(item.audits[0].result.has_more).toBe(true);
    expect(item.audits[0].result.next_line).toBe(4);
    expect(item.answer).toContain(item.expected[0].replace(/^#+\s*/, ''));
  }
  writeFileSync('test-results/skill-reader-smoke.json', JSON.stringify(report,null,2));
  console.log(JSON.stringify(report.map(({mode,prompts,answer}) => ({mode,prompts,answer}))));
  await page.reload();
} finally { await browser.close(); }
