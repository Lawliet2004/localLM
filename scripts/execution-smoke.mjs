import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
const folder=resolve('.local',`execution-smoke-${randomUUID()}`);
mkdirSync(folder,{recursive:true}); writeFileSync(join(folder,'numbers.txt'),'17 25 12');
const browser=await chromium.connectOverCDP('http://127.0.0.1:9223');
const page=browser.contexts()[0].pages().find(value=>value.url().includes('1420'));
if(!page) throw new Error('Native LocalLM webview is unavailable.');
page.setDefaultTimeout(15000);
let previousWorkspace;
try {
  previousWorkspace=await page.evaluate(async()=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js'); return (await invoke('get_workspace')).path;});
  await page.evaluate(async path=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js'); await invoke('set_workspace',{path});},folder);
  await page.reload();
  await page.getByRole('button',{name:'Execution',exact:true}).click();
  await expect(page.getByRole('textbox',{name:'Python executable',exact:true})).not.toHaveValue('');
  await page.getByRole('button',{name:'Save interpreters',exact:true}).click();
  await expect(page.getByText('Interpreter settings saved.',{exact:true})).toBeVisible();
  await page.getByRole('button',{name:'Models & runtime',exact:true}).click();
  if(await page.getByRole('button',{name:'Load model',exact:true}).count()) { await page.getByRole('button',{name:'Load model',exact:true}).click(); await expect(page.getByRole('button',{name:'Unload',exact:true})).toBeVisible({timeout:120000}); }
  const scenarios=[];
  for(const mode of ['allow','deny','cancel']) {
    await page.getByRole('button',{name:'New conversation',exact:false}).click();
    const picker=page.locator('.tool-picker'); if(await picker.getAttribute('open')===null) await picker.locator('summary').first().click();
    await page.getByRole('checkbox',{name:'Local code',exact:true}).check();
    const code=mode==='allow' ? "from pathlib import Path\nprint(sum(map(int, Path('numbers.txt').read_text().split())))" : mode==='deny' ? "from pathlib import Path\nPath('denied.txt').write_text('should not exist')" : "from pathlib import Path\nimport time\nPath('started.txt').write_text('started')\ntime.sleep(4)\nPath('after-cancel.txt').write_text('should not exist')";
    await page.getByRole('textbox',{name:'Message',exact:true}).fill(`Use run_code with language python to run exactly the following code. Do not modify the code. If denied, stop.\n\n\`\`\`python\n${code}\n\`\`\``);
    await page.getByRole('button',{name:'Send message',exact:true}).click();
    const dialog=page.getByRole('dialog'); await expect(dialog).toBeVisible({timeout:180000});
    await expect(dialog.getByText('run_code',{exact:true})).toBeVisible();
    const args=JSON.parse(await dialog.locator('.approval-arguments').textContent());
    expect(args.language).toBe('python'); expect(args.code.trim()).toBe(code);
    await dialog.getByRole('button',{name:mode==='deny'?'Deny':'Allow once',exact:true}).click();
    await expect(dialog).toHaveCount(0,{timeout:15000});
    if(mode==='cancel') { await expect.poll(()=>existsSync(join(folder,'started.txt'))).toBe(true); await page.getByRole('button',{name:'Stop response',exact:true}).click(); }
    await expect(page.getByRole('button',{name:'Stop response'})).toHaveCount(0,{timeout:180000});
    await expect(page.getByRole('alert')).toHaveCount(0);
    const answer=await page.locator('.message-assistant .markdown').innerText();
    const records=await page.evaluate(async()=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js'); const state=await invoke('bootstrap'); return (await invoke('get_messages',{id:state.conversations[0].id})).filter(message=>message.role==='tool').map(message=>({status:message.status,body:JSON.parse(message.content)}));});
    if(mode==='allow') { expect(records[0].body.result.exitCode).toBe(0); expect(records[0].body.result.stdout.trim()).toBe('54'); expect(answer).toContain('54'); }
    if(mode==='deny') expect(existsSync(join(folder,'denied.txt'))).toBe(false);
    if(mode==='cancel') { await new Promise(resolve=>setTimeout(resolve,4500)); expect(existsSync(join(folder,'after-cancel.txt'))).toBe(false); expect(records[0].status).toBe('interrupted'); }
    scenarios.push({mode,args,answer,records});
  }
  mkdirSync('test-results',{recursive:true}); await page.screenshot({path:'test-results/native-execution.png'});
  const report={testedAt:new Date().toISOString(),scenarios}; writeFileSync('test-results/execution-smoke.json',JSON.stringify(report,null,2)); console.log(JSON.stringify(report));
} finally {
  await page.evaluate(async()=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js'); await invoke('cancel_generation');}).catch(()=>{});
  if(previousWorkspace!==undefined) await page.evaluate(async path=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js'); await invoke('set_workspace',{path});},previousWorkspace).catch(()=>{});
  await browser.close();
}
