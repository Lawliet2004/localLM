import { chromium, expect } from '@playwright/test';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
const directory = mkdtempSync(join(tmpdir(), 'locallm-mcp-smoke-'));
const name = `Local fixture ${Date.now()}`;
const script = join(directory, 'server.cjs');
writeFileSync(script, `
require('node:readline').createInterface({input:process.stdin}).on('line', line => {
 const r = JSON.parse(line); if (r.id === undefined) return;
 const result = r.method === 'initialize'
 ? {protocolVersion:r.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'fixture',version:'1'}}
 : {tools:[{name:'fixture_echo',description:'Return fixture text',inputSchema:{type:'object'}}]};
 process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,result})+'\\n');
});`);
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
let id;
const invoke = (command, args = {}) => page.evaluate(async ({command,args}) => {
  const {invoke} = await import('/node_modules/@tauri-apps/api/core.js');
  return invoke(command,args);
}, {command,args});
try {
  await expect.poll(() => { page = browser.contexts()[0].pages().find(value => value.url().includes('1420')); return Boolean(page); }).toBe(true);
  await page.getByRole('button', {name:'Connectors',exact:true}).click();
  await page.getByText('Add a local MCP server', {exact:true}).click();
  await page.getByLabel('Name', {exact:true}).fill(name);
  await page.getByLabel('Executable path', {exact:true}).fill(process.execPath);
  await page.getByLabel('Working directory', {exact:true}).fill(directory);
  await page.getByLabel('Arguments (JSON array)', {exact:true}).fill(JSON.stringify([script]));
  await page.getByLabel('Environment variables (JSON object)', {exact:true}).fill(JSON.stringify({FIXTURE_KEY:'test-value'}));
  await page.getByRole('button', {name:'Save local server',exact:true}).click();
  await expect(page.getByRole('status')).toContainText('Saved.');
  const saved = (await invoke('list_connectors')).find(item => item.description === name);
  expect(saved.connected).toBe(false); id = saved.id;
  await expect(page.getByLabel('Environment variables (JSON object)', {exact:true})).toHaveValue('{}');
  const card = page.locator('details.catalog-item').filter({has:page.locator('strong', {hasText:name})});
  await card.locator(':scope > summary').click();
  await card.getByRole('button', {name:'Connect',exact:true}).click();
  await expect(card.getByText('1 tools', {exact:true})).toBeVisible();
  expect((await invoke('list_connectors')).find(item => item.id === id).tools[0].name).toBe('fixture_echo');
  await page.screenshot({path:'test-results/local-connector-smoke.png',fullPage:true});
  await card.getByRole('button', {name:'Disconnect',exact:true}).click();
  await expect(card.getByText('Not connected', {exact:true})).toBeVisible();
  await card.getByRole('button', {name:'Edit configuration',exact:true}).click();
  await expect(page.getByLabel('Name', {exact:true})).toHaveValue(name);
  await expect(page.getByLabel('Arguments (JSON array)', {exact:true})).toHaveValue(JSON.stringify([script]));
  await expect(page.getByLabel('Environment variables (JSON object)', {exact:true})).toHaveValue(JSON.stringify({FIXTURE_KEY:'test-value'}));
  await page.getByLabel('Name', {exact:true}).fill('Unsaved change');
  await page.getByLabel('Environment variables (JSON object)', {exact:true}).fill('{}');
  await page.getByRole('button', {name:'Cancel editing',exact:true}).click();
  const afterCancel = await invoke('read_local_connector', {id});
  expect(afterCancel.name).toBe(name);
  expect(afterCancel.environment).toEqual({FIXTURE_KEY:'test-value'});
  await card.getByRole('button', {name:'Edit configuration',exact:true}).click();
  await page.getByLabel('Name', {exact:true}).fill(`${name} renamed`);
  await page.getByRole('button', {name:'Save local server',exact:true}).click();
  await expect.poll(async () => (await invoke('list_connectors')).find(item => item.id === id)?.description).toBe(`${name} renamed`);
  const edited = await invoke('read_local_connector', {id});
  expect(edited.arguments).toEqual([script]);
  expect(edited.environment).toEqual({FIXTURE_KEY:'test-value'});
  expect(edited.executable).toBe(process.execPath);
  expect(edited.workingDirectory).toBe(directory);
  await expect(page.getByLabel('Environment variables (JSON object)', {exact:true})).toHaveValue('{}');
  const renamed = page.locator('details.catalog-item').filter({has:page.locator('strong', {hasText:`${name} renamed`})});
  await renamed.getByRole('button', {name:'Connect',exact:true}).click();
  await expect(renamed.getByText('1 tools', {exact:true})).toBeVisible();
  await renamed.getByRole('button', {name:'Disconnect',exact:true}).click();
  await renamed.getByRole('button', {name:'Remove server',exact:true}).click();
  await expect(renamed).toHaveCount(0);
  expect((await invoke('list_local_connectors')).some(item => item.id === id)).toBe(false);
  writeFileSync('test-results/local-connector-smoke.json', JSON.stringify({testedAt:new Date().toISOString(),savedWithoutLaunching:true,discoveredTool:'fixture_echo',cancelPreservedConfiguration:true,editedSameId:true,connectionSettingsPreserved:true,reconnectedAfterEdit:true,disconnectedAndRemoved:true},null,2));
} finally {
  if (page) {
    // Recover only this fixture if an assertion failed before its ID was read.
    id ??= (await invoke('list_connectors')).find(item => item.description === name)?.id;
    if (id && (await invoke('list_local_connectors')).some(item => item.id === id)) {
      await invoke('disconnect_connector', {id,forget:false});
      await invoke('remove_local_connector', {id});
    }
  }
  await browser.close();
  rmSync(directory,{recursive:true,force:true});
}
