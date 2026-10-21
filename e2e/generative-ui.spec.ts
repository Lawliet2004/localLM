import { test,expect,type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
async function setup(page:Page,scenario?:string){
 if(scenario)await page.addInitScript(value=>sessionStorage.setItem("visual-scenario",value),scenario);
 await page.route('**/src/lib/api.ts*',route=>route.fulfill({contentType:'application/javascript',body:readFileSync('e2e/fixtures/audit-api.js','utf8')+'\n'+readFileSync('e2e/fixtures/generative-ui-api.js','utf8')}));
 await page.route(/.*@tauri-apps_api_window\.js.*/,route=>route.fulfill({contentType:'application/javascript',body:'export const getCurrentWindow=()=>({isMaximized:async()=>false,onResized:async()=>()=>{}})'}));
 await page.goto('/');await page.getByText('Visual acceptance',{exact:true}).click();
}
test('saved visual answers render without a composer mode selector',async({page})=>{
 await setup(page);await expect(page.getByRole('group',{name:'SQL joins'})).toBeVisible();
 await expect(page.getByRole('combobox',{name:'Response mode'})).toHaveCount(0);
});
test('quiz state survives history reload and follow-ups use ordinary submission',async({page})=>{
 await setup(page);await page.getByRole('radio',{name:'Left',exact:true}).check();await expect(page.getByText('Score: 1 / 1')).toBeVisible();
 await expect.poll(()=>page.evaluate(()=>{const raw=sessionStorage.getItem('visual-state');return raw ? JSON.parse(raw)?.joins?.value?.['0'] : null;})).toBe(1);
 await page.reload();await page.getByText('Visual acceptance',{exact:true}).click();await expect(page.getByRole('radio',{name:'Left',exact:true})).toBeChecked();
 await page.getByRole('button',{name:'Explain further',exact:true}).click();await expect.poll(()=>page.evaluate(()=>(window as unknown as {__audit:{sends:number}}).__audit.sends)).toBe(1);
});
test('original answer and raw source remain reviewable',async({page})=>{
 await setup(page);await page.getByRole('button',{name:'Original answer',exact:true}).click();await expect(page.getByText('LEFT JOIN preserves every left row.',{exact:true})).toBeVisible();
 await page.getByText('UI source',{exact:true}).click();await expect(page.locator('pre').filter({hasText:'root = Response'})).toBeVisible();
});

for(const scenario of ['invalid','interrupted'])test(`${scenario} visual output keeps original text readable`,async({page})=>{
 await setup(page,scenario);await expect(page.getByText('LEFT JOIN preserves every left row.',{exact:true})).toBeVisible();
 await expect(page.getByRole('group',{name:'SQL joins'})).toHaveCount(0);
 if(scenario==='invalid')await expect.poll(()=>page.evaluate(()=>sessionStorage.getItem('visual-rejection'))).not.toBeNull();
 await page.reload();await page.getByText('Visual acceptance',{exact:true}).click();await expect(page.getByText('LEFT JOIN preserves every left row.',{exact:true})).toBeVisible();
});
test('renderer exceptions persist rejection and preserve the original answer',async({page})=>{
 await page.route('**/src/genui/Chart.tsx*',route=>route.fulfill({contentType:'application/javascript',body:'export function Chart(){throw new Error("Controlled test component failure");}'}));
 await setup(page,'render-error');await expect(page.getByText('Jan sales: 10',{exact:true})).toBeVisible();
 await expect.poll(()=>page.evaluate(()=>{const raw=sessionStorage.getItem('visual-rejection');return raw?JSON.parse(raw).messageId:null;})).toBe('visual-answer');
});
