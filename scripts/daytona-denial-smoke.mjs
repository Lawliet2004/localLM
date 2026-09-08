import {chromium,expect} from '@playwright/test';
import {randomUUID} from 'node:crypto';
import {writeFileSync} from 'node:fs';
const browser=await chromium.connectOverCDP('http://127.0.0.1:9223');let page;
await expect.poll(()=>{page=browser.contexts()[0].pages().find(p=>p.url().includes('1420'));return Boolean(page);}).toBe(true);
async function invoke(command,args={}){return page.evaluate(async({command,args})=>{const{invoke}=await import('/node_modules/@tauri-apps/api/core.js');return invoke(command,args);},{command,args});}
const original=await invoke('bootstrap');let saved=false;const chats=[];
try {
  if(await invoke('has_daytona_key'))throw new Error('Refusing to replace an existing Daytona credential.');
  expect(await invoke('pending_daytona_operations')).toEqual([]);
  await invoke('save_daytona_key',{key:'denial-fixture-'+randomUUID()});saved=true;
  await invoke('save_preferences',{preferences:{...original.preferences,maxTokens:2048,temperature:0}});
  if(original.runtime.phase!=='ready')await invoke('load_model');
  const report=[];
  for(const accessMode of ['ask','autoApprove']) {
    const chat=await invoke('create_conversation');chats.push(chat.id);const title='Daytona denial '+randomUUID().slice(0,8);
    await invoke('rename_conversation',{id:chat.id,title});
    await invoke('save_conversation_tools',{id:chat.id,tools:{sources:['__daytona'],tools:[],accessMode}});
    await page.reload();await page.getByRole('button',{name:title,exact:true}).click();
    await page.locator('.tool-picker > summary').click();
    await expect(page.getByRole('checkbox',{name:'Daytona cloud code'})).toBeChecked();
    await page.getByRole('textbox',{name:'Message',exact:true}).fill('Use the Daytona run_code tool exactly once: language python, code print(42), timeout_seconds 30. If denied, stop and do not try another tool.');
    await page.getByRole('button',{name:'Send message',exact:true}).click();
    const dialog=page.getByRole('dialog');await expect(dialog).toBeVisible({timeout:120000});
    await expect(dialog).toContainText('temporary Daytona cloud sandbox');
    await expect(dialog.locator('.approval-code')).toContainText('print(42)');
    await dialog.getByRole('button',{name:'Deny',exact:true}).click();
    await expect(page.getByRole('button',{name:'Stop response'})).toHaveCount(0,{timeout:120000});
    const audits=(await invoke('get_messages',{id:chat.id})).filter(m=>m.role==='tool').map(m=>JSON.parse(m.content));
    expect(audits).toHaveLength(1);expect(audits[0].request.connector).toBe('Daytona');expect(audits[0].request.decision).toBe('denied');
    expect(await invoke('pending_daytona_operations')).toEqual([]);
    report.push({accessMode,decision:'denied',resourceRecords:0});
  }
  writeFileSync('test-results/daytona-denial-smoke.json',JSON.stringify(report,null,2));console.log(JSON.stringify(report));
} finally {
  await invoke('save_preferences',{preferences:original.preferences});
  for(const id of chats)await invoke('delete_conversation',{id});
  if(saved)await invoke('forget_daytona_key');
  if(original.runtime.phase!=='ready')await invoke('unload_model');
  await page.reload();await browser.close();
}
