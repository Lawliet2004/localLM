import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { randomUUID } from 'node:crypto';
const browser=await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;await expect.poll(()=>{page=browser.contexts()[0].pages().find(page=>page.url().includes('1420'));return Boolean(page);}).toBe(true);
page.setDefaultTimeout(15000);
async function invoke(command,args={}){return page.evaluate(async({command,args})=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js');return invoke(command,args);},{command,args});}
const original=(await invoke('bootstrap')).preferences;
const workspace=await invoke('get_workspace');
const folder=resolve('.local','context-smoke-'+randomUUID());mkdirSync(folder,{recursive:true});
const oversized='word '.repeat(18000);writeFileSync(join(folder,'long.txt'),oversized);
try{
  await invoke('save_preferences',{preferences:{...original,maxTokens:2048,temperature:0}});
  await page.getByRole('button',{name:'Models & runtime',exact:true}).click();
  if(await page.getByRole('button',{name:'Load model',exact:true}).count()){
    await page.getByRole('button',{name:'Load model',exact:true}).click();
    await expect(page.getByRole('button',{name:'Unload',exact:true})).toBeVisible({timeout:120000});
  }
  await page.getByRole('button',{name:/New conversation/}).click();
  // Keep UI input within its 100,000-character limit while exceeding 8,192 tokens.
  const prompt='word '.repeat(12000);
  await page.getByRole('textbox',{name:'Message',exact:true}).fill(prompt);
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.getByRole('alert')).toContainText('Context limit exceeded',{timeout:30000});
  await expect(page.getByRole('button',{name:'Stop response'})).toHaveCount(0);
  await expect(page.getByRole('textbox',{name:'Message',exact:true})).toHaveValue(prompt);
  let state=await invoke('bootstrap');
  expect(await invoke('get_messages',{id:state.conversations[0].id})).toHaveLength(0);
  const preflightError=await page.getByRole('alert').innerText();
  await page.getByRole('textbox',{name:'Message',exact:true}).fill('Reply with just OK.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.getByRole('button',{name:'Stop response'})).toHaveCount(0,{timeout:180000});
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.locator('.message-assistant .markdown')).toContainText('OK');
  await invoke('set_workspace',{path:folder});await page.reload();
  await page.getByRole('button',{name:/New conversation/}).click();
  await page.getByRole('combobox',{name:'Permission mode'}).selectOption('autoApprove');
  await page.locator('.tool-picker > summary').click();
  await page.getByRole('checkbox',{name:'Workspace files',exact:true}).check();
  await page.getByRole('textbox',{name:'Message',exact:true}).fill('Use read_file once with path long.txt. Read its first line. Do not list files first.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.getByRole('alert')).toContainText('Context limit exceeded',{timeout:180000});
  await expect(page.getByRole('button',{name:'Stop response'})).toHaveCount(0);
  state=await invoke('bootstrap');const rows=await invoke('get_messages',{id:state.conversations[0].id});
  expect(rows.find(row=>row.role==='assistant').error).toContain('Context limit exceeded');
  const audit=rows.filter(row=>row.role==='tool').map(row=>JSON.parse(row.content)).find(audit=>audit.request.name==='read_file');
  expect(audit.result.isError).not.toBe(true);
  expect(audit.result.content.length).toBeGreaterThan(85000);
  const report={testedAt:new Date().toISOString(),preflightError,draftRestored:true,noOversizedMessageSaved:true,shortMessageWorks:true,toolResultOverflowStopped:true};
  writeFileSync('test-results/context-smoke.json',JSON.stringify(report,null,2));console.log(JSON.stringify(report));
}finally{
  await invoke('cancel_generation');await expect(page.getByRole('button',{name:'Stop response'})).toHaveCount(0,{timeout:20000});
  await invoke('save_preferences',{preferences:original});await invoke('set_workspace',{path:workspace.path});await browser.close();
}
