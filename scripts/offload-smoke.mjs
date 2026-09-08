import {chromium,expect} from '@playwright/test';
import {writeFileSync} from 'node:fs';
const browser=await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(()=>{page=browser.contexts()[0].pages().find(item=>item.url().includes('1420'));return Boolean(page);}).toBe(true);
async function invoke(command,args={}) {return page.evaluate(async({command,args})=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js');return invoke(command,args);},{command,args});}
const original=await invoke('bootstrap');
const report=[];
try {
  for(const gpuLayers of [-1,5,0]) {
    await invoke('unload_model');
    await invoke('save_runtime_config',{config:{...original.config,gpuLayers}});
    const status=await invoke('load_model');
    expect(status.phase).toBe('ready');
    expect(status.gpuOffload).not.toBeNull();
    expect(status.gpuOffload.layers).toBe(gpuLayers===-1?status.gpuOffload.totalLayers:gpuLayers);
    report.push({requested:gpuLayers,reported:status.gpuOffload});
    await page.reload();
    await page.getByRole('button',{name:'Models & runtime',exact:true}).click();
    await expect(page.getByText(`Runtime reports ${status.gpuOffload.layers} / ${status.gpuOffload.totalLayers} model layers offloaded to GPU.`,{exact:false})).toBeVisible();
    if(gpuLayers===-1) await page.screenshot({path:'test-results/offload.png'});
  }
  writeFileSync('test-results/offload-smoke.json',JSON.stringify(report,null,2));
  console.log(JSON.stringify(report));
} finally {
  await invoke('unload_model');
  await invoke('save_runtime_config',{config:original.config});
  if(original.runtime.phase==='ready') await invoke('load_model');
  await page.reload();
  await browser.close();
}
