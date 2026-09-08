import { chromium, expect } from '@playwright/test';
import { randomUUID } from 'node:crypto';
import { writeFileSync } from 'node:fs';
const browser=await chromium.connectOverCDP('http://127.0.0.1:9223');
const page=browser.contexts()[0].pages().find(page=>page.url().includes('1420'));
page.setDefaultTimeout(15000);
async function invoke(command,args={}){return page.evaluate(async({command,args})=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js');return invoke(command,args);},{command,args});}
const originalNew=await page.evaluate(()=>localStorage.getItem('locallm-draft:new'));
const chats=[];const suffix=randomUUID().slice(0,8);
try{
 for(const name of ['A','B']){const chat=await invoke('create_conversation');chats.push(chat);await invoke('rename_conversation',{id:chat.id,title:`Draft ${name} ${suffix}`});}
 await page.reload();
 const input=page.getByRole('textbox',{name:'Message',exact:true});
 await page.getByRole('button',{name:`Draft A ${suffix}`,exact:true}).click();await input.fill('Unsent alpha');
 await page.getByRole('button',{name:`Draft B ${suffix}`,exact:true}).click();await expect(input).toHaveValue('');await input.fill('Unsent beta');
 await page.getByRole('button',{name:`Draft A ${suffix}`,exact:true}).click();await expect(input).toHaveValue('Unsent alpha');
 await page.getByRole('button',{name:/New conversation/}).click();await input.fill('New conversation draft');
 await page.reload();await expect(input).toHaveValue('New conversation draft');
 await page.getByRole('button',{name:`Draft B ${suffix}`,exact:true}).click();await expect(input).toHaveValue('Unsent beta');
 await page.getByRole('button',{name:'Models & runtime',exact:true}).click();await page.getByRole('button',{name:'Conversations',exact:true}).click();await expect(input).toHaveValue('Unsent beta');
 const report={testedAt:new Date().toISOString(),isolatedDrafts:true,reload:true,pageNavigation:true,newChatDraft:true};writeFileSync('test-results/drafts-smoke.json',JSON.stringify(report,null,2));console.log(JSON.stringify(report));
}finally{
 for(const chat of chats)await invoke('delete_conversation',{id:chat.id});
 await page.evaluate(({ids,originalNew})=>{for(const id of ids)localStorage.removeItem('locallm-draft:'+id);if(originalNew===null)localStorage.removeItem('locallm-draft:new');else localStorage.setItem('locallm-draft:new',originalNew);},{ids:chats.map(chat=>chat.id),originalNew});
 await page.reload();await browser.close();
}
