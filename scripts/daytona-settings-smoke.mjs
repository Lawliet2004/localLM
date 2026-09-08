import {chromium,expect} from '@playwright/test';
import {randomUUID} from 'node:crypto';
import {readFileSync,writeFileSync} from 'node:fs';
import {join} from 'node:path';
const browser=await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(()=>{page=browser.contexts()[0].pages().find(item=>item.url().includes('1420'));return Boolean(page);}).toBe(true);
async function invoke(command,args={}){return page.evaluate(async({command,args})=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js');return invoke(command,args);},{command,args});}
let saved=false;
try {
  if(await invoke('has_daytona_key')) throw new Error('Existing Daytona credential: refusing to replace it for a test.');
  const key='locallm-fixture-'+randomUUID();
  await page.getByRole('button',{name:'Execution',exact:true}).click();
  await page.getByLabel('Daytona API key').fill(key);
  await page.getByRole('button',{name:'Save Daytona key',exact:true}).click();
  await expect.poll(()=>invoke('has_daytona_key')).toBe(true);saved=true;
  await expect(page.getByLabel('Daytona API key')).toHaveValue('');
  expect(readFileSync(join(process.env.APPDATA,'app.locallm.desktop','credentials','daytona.sealed')).includes(Buffer.from(key))).toBe(false);
  await expect(page.getByText('Daytona key saved securely. Account access has not been verified.')).toBeVisible();
  await page.reload();
  await page.getByRole('button',{name:'Execution',exact:true}).click();
  await expect(page.getByText('An encrypted API key is saved.',{exact:false})).toBeVisible();
  await expect(page.getByLabel('Daytona API key')).toHaveValue('');
  await page.getByRole('button',{name:'Forget Daytona key',exact:true}).click();
  await expect.poll(()=>invoke('has_daytona_key')).toBe(false);saved=false;
  await expect(page.getByText('Daytona key removed.')).toBeVisible();
  const report={testedAt:new Date().toISOString(),saved:true,restoredPresenceAfterReload:true,secretNotReturnedToInput:true,forgotten:true,cloudCalls:0};
  writeFileSync('test-results/daytona-settings-smoke.json',JSON.stringify(report,null,2));console.log(JSON.stringify(report));
} finally {if(saved)await invoke('forget_daytona_key');await browser.close();}
