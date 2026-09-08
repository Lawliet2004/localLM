import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(() => { page = browser.contexts()[0].pages().find(page => page.url().includes('1420')); return Boolean(page); }).toBe(true);
page.setDefaultTimeout(15000);
async function invoke(command, args = {}) { return page.evaluate(async ({command,args}) => { const {invoke}=await import('/node_modules/@tauri-apps/api/core.js'); return invoke(command,args); },{command,args}); }
const original = (await invoke('bootstrap')).preferences;
let id;
try {
  await page.getByRole('button', {name:'Models & runtime',exact:true}).click();
  if(await page.getByRole('button',{name:'Load model',exact:true}).count()) {
    await page.getByRole('button',{name:'Load model',exact:true}).click();
    await expect(page.getByRole('button',{name:'Unload',exact:true})).toBeVisible({timeout:120000});
  }
  await invoke('save_preferences',{preferences:{...original,maxTokens:1,temperature:0}});
  id=(await invoke('create_conversation')).id;
  const failure=await page.evaluate(async id=>{
    const {invoke,Channel}=await import('/node_modules/@tauri-apps/api/core.js');
    try { await invoke('send_message',{conversationId:id,content:'Explain how Rust ownership works in detail.',connectorIds:[],connectorTools:[],channel:new Channel()});return ''; }
    catch(error){return String(error);}
  },id);
  expect(failure).toContain('Response token limit reached');
  const messages=await invoke('get_messages',{id});
  expect(messages).toHaveLength(2);
  expect(messages[1].status).toBe('error');
  expect(messages[1].error).toBe(failure);
  const title='Response limit check '+randomUUID().slice(0,8);
  await invoke('rename_conversation',{id,title});
  await page.reload();
  await page.getByRole('button',{name:title,exact:true}).click();
  await expect(page.locator('.message-state')).toHaveText(failure);
  const report={testedAt:new Date().toISOString(),error:failure,persisted:true,visibleAfterReload:true,messageCount:messages.length};
  writeFileSync('test-results/response-limit-smoke.json',JSON.stringify(report,null,2));console.log(JSON.stringify(report));
} finally { await invoke('save_preferences',{preferences:original});await browser.close(); }
