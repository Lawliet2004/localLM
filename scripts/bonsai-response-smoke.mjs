import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync, unlinkSync } from 'node:fs';
const browser = await chromium.launch({headless:true,channel:'msedge'});
const page = await browser.newPage({viewport:{width:1120,height:900}});
const errors=[]; page.on('pageerror',e=>errors.push(e.message));
mkdirSync('.vitest',{recursive:true});
try {
  await page.route(/\/src\/main\.tsx(?:\?.*)?$/,r=>r.fulfill({contentType:'application/javascript',body:''}));
  await page.route(/\/src\/lib\/api\.ts(?:\?.*)?$/,r=>r.fulfill({contentType:'application/javascript',body:`
    export const nativeAvailable=true; export const errorMessage=String;
    export const api={getConversationRun:async()=>({id:'run',conversationId:'chat',status:'completed',createdAt:1000,updatedAt:232000}),
      modelDownloadInfo:async(filename)=>({filename,destination:'C:/models/'+filename,bytes:2182184672,sha256:'verified-test-fixture',availableBytes:10000000000,requiredBytes:3000000000,destinationExists:false}),
      modelInstallStatus:async()=>({busy:false,phase:'',path:null,received:0,total:0}),
      installModel:async(filename)=>{window.requestedModel=filename;return {busy:false,phase:'ready',path:'C:/models/'+filename,received:2182184672,total:2182184672};}};
  `}));
  await page.goto('http://localhost:1420');
  writeFileSync('.vitest/response-probe.js',`
    import React from 'react'; import {createRoot} from 'react-dom/client';
    import {Chat} from '/src/components/Chat.tsx';
    import {ModelSelectorPanel} from '/src/components/ModelSelectorPanel.tsx';
    import {ModelDownload} from '/src/components/ModelDownload.tsx';
    import '/src/styles.css';
    const base={conversationId:'chat',status:'complete',reasoning:'',createdAt:1000};
    const messages=[{...base,id:'u',role:'user',content:'Move the tool controls into their own section.'},
      {...base,id:'a',role:'assistant',content:'Moved the controls into a dedicated **Tools** sidebar section beside Connectors.\\n\\nIt includes clear setup guidance, unavailable-tool warnings, and automatic saving.\\n\\n**Verified:** the targeted tests and browser checks passed.'},
      {...base,id:'t',role:'tool',content:JSON.stringify({request:{connector:'Workspace',name:'edit_file',arguments:{path:'src/styles.css'}},result:{diff:'--- a/src/styles.css\\n+++ b/src/styles.css\\n@@ -1 +1,2 @@\\n-old rule\\n+new rule\\n+responsive rule\\n'}})}];
    function Probe(){const [filename,setFilename]=React.useState(''); return React.createElement('div',{className:'app sidebar-collapsed'},React.createElement('main',{className:'workspace'},filename
      ? React.createElement('div',{className:'settings-page'},React.createElement(ModelSelectorPanel,{providers:[],selection:{providerId:null,modelId:''},busy:false,onSave:async()=>{},localFilename:filename,onLocalModelChange:setFilename}),React.createElement(ModelDownload,{busy:false,filename,onSelect:()=>{}}))
      : React.createElement(Chat,{messages,conversationKey:'chat',generating:false,ready:true,loading:false,onSend:async()=>{},onCancel(){},onConfigure(){},providers:[],onSelectModel:async()=>{},onConfigureLocalModel:setFilename})))}
    createRoot(document.getElementById('root')).render(React.createElement(Probe));
  `);
  await page.evaluate(()=>import('/.vitest/response-probe.js'));
  await expect(page.getByText('Worked for 3m 51s')).toBeVisible();
  await expect(page.getByText('Edited styles.css')).toBeVisible();
  await page.getByText('Review',{exact:true}).click();
  await expect(page.locator('.file-change-review pre')).toBeVisible();
  await page.getByText('Review',{exact:true}).click();
  await page.screenshot({path:'.vitest/work-summary-desktop.png'});
  await page.getByRole('button',{name:'Local model',exact:true}).click();
  await page.getByRole('button',{name:/Ternary Bonsai 8B/}).click();
  await expect(page.getByRole('combobox',{name:'Local model option'})).toHaveValue('Ternary-Bonsai-8B-Q2_0.gguf');
  await expect(page.getByText(/Requires a Prism/)).toBeVisible();
  await page.getByRole('button',{name:'Download model'}).click();
  expect(await page.evaluate(()=>window.requestedModel)).toBe('Ternary-Bonsai-8B-Q2_0.gguf');
  await page.screenshot({path:'.vitest/bonsai-selector.png'});
  expect(errors).toEqual([]);
  console.log('Browser passed: persisted-turn summary, actual diff counts and Review, exact Bonsai selector and download request; mock backend.');
} finally {await browser.close(); unlinkSync('.vitest/response-probe.js');}
