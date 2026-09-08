import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9224');
let page;
await expect.poll(() => { page=browser.contexts()[0].pages().find(item=>item.url().includes('tauri.localhost')); return Boolean(page); }).toBe(true);
const errors=[];
page.on('pageerror',error=>errors.push(error.message));
async function invoke(command,args={}) { return page.evaluate(({command,args})=>window.__TAURI_INTERNALS__.invoke(command,args),{command,args}); }
let chat;
const original=(await invoke('bootstrap')).preferences;
try {
  await expect(page.getByText('Browser preview',{exact:false})).toHaveCount(0);
  await page.getByRole('button',{name:'Models & runtime',exact:true}).click();
  await page.getByRole('button',{name:'Load model',exact:true}).click();
  await expect(page.getByRole('button',{name:'Unload',exact:true})).toBeVisible({timeout:120000});
  await invoke('save_preferences',{preferences:{...original,temperature:0,maxTokens:2048}});
  chat=await invoke('create_conversation');
  await invoke('rename_conversation',{id:chat.id,title:'Installed release smoke'});
  await page.reload();
  await page.getByRole('button',{name:'Installed release smoke',exact:true}).click();
  await page.getByRole('textbox',{name:'Message',exact:true}).fill('What is 17 + 25? Answer briefly.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.getByRole('button',{name:'Stop response'})).toBeVisible();
  await expect(page.getByRole('button',{name:'Stop response'})).toHaveCount(0,{timeout:180000});
  await expect(page.locator('.message-assistant .markdown')).toContainText('42');
  await expect(page.getByRole('alert')).toHaveCount(0);
  const messages=await invoke('get_messages',{id:chat.id});
  expect(messages).toHaveLength(2);
  expect(messages[1].status).toBe('complete');
  expect(errors).toEqual([]);
  await page.screenshot({path:'test-results/installed-release.png'});
  const report={testedAt:new Date().toISOString(),url:page.url(),answer:messages[1].content,messageStatus:messages[1].status,errors};
  writeFileSync('test-results/release-smoke.json',JSON.stringify(report,null,2));
  console.log(JSON.stringify(report));
} finally {
  await invoke('save_preferences',{preferences:original});
  if(chat) await invoke('delete_conversation',{id:chat.id});
  await invoke('unload_model');
  await browser.close();
}
