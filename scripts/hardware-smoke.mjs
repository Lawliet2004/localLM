import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
const browser=await chromium.connectOverCDP('http://127.0.0.1:9223');
const page=browser.contexts()[0].pages().find(value=>value.url().includes('1420'));
if(!page) throw new Error('Native LocalLM webview is unavailable.');
page.setDefaultTimeout(15000);
try {
  await page.getByRole('button',{name:'Models & runtime',exact:true}).click();
  const panel=page.getByRole('region',{name:'Hardware status'});
  await expect(panel.getByText('NVIDIA GeForce RTX 2050',{exact:true})).toBeVisible();
  await expect(panel.getByRole('progressbar',{name:'NVIDIA GeForce RTX 2050 VRAM usage'})).toBeVisible();
  const before=await page.evaluate(async()=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js'); return invoke('hardware_status');});
  if(await page.getByRole('button',{name:'Load model',exact:true}).count()) { await page.getByRole('button',{name:'Load model',exact:true}).click(); await expect(page.getByRole('button',{name:'Unload',exact:true})).toBeVisible({timeout:120000}); }
  await expect(panel.getByText('Automatic GPU fill requested',{exact:false})).toBeVisible();
  const changed=await page.evaluate(async()=>{
    const {invoke}=await import('/node_modules/@tauri-apps/api/core.js'); const initial=await invoke('bootstrap');
    await invoke('save_runtime_config',{config:{...initial.config,gpuLayers:0}});
    try { return await invoke('bootstrap'); } finally { await invoke('save_runtime_config',{config:initial.config}); }
  });
  expect(changed.config.gpuLayers).toBe(0); expect(changed.runtime.loadedConfig.gpuLayers).toBe(-1);
  await page.getByRole('button',{name:'New conversation',exact:false}).click();
  await page.getByRole('textbox',{name:'Message',exact:true}).fill('Write a detailed 1000-word explanation of Rust ownership with several code examples and a comparison of shared and mutable borrowing.');
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.getByRole('button',{name:'Stop response'})).toBeVisible();
  const samples=[];
  for(let index=0;index<8;index++) {
    samples.push(await page.evaluate(async()=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js'); return invoke('hardware_status');}));
    await new Promise(resolve=>setTimeout(resolve,350));
  }
  expect(samples.some(sample=>sample.gpus.some(gpu=>gpu.utilizationPercent>0))).toBe(true);
  if(await page.getByRole('button',{name:'Stop response'}).count()) await page.getByRole('button',{name:'Stop response'}).click();
  await expect(page.getByRole('button',{name:'Stop response'})).toHaveCount(0,{timeout:15000});
  await page.getByRole('button',{name:'Models & runtime',exact:true}).click();
  await expect(panel.getByText('NVIDIA GeForce RTX 2050',{exact:true})).toBeVisible();
  mkdirSync('test-results',{recursive:true}); await page.screenshot({path:'test-results/native-hardware.png'});
  const report={testedAt:new Date().toISOString(),before,samples}; writeFileSync('test-results/hardware-smoke.json',JSON.stringify(report,null,2));
  console.log(JSON.stringify({gpu:before.gpus[0].name,logicalCpus:before.logicalCpus,memoryTotalBytes:before.memoryTotalBytes,utilizationSamples:samples.map(sample=>sample.gpus[0].utilizationPercent),memorySamples:samples.map(sample=>sample.gpus[0].memoryUsedMib)}));
} finally { await page.evaluate(async()=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js'); await invoke('cancel_generation');}).catch(()=>{}); await browser.close(); }
