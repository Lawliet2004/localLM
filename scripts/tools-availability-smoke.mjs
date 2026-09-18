import { chromium, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';

const browser = await chromium.launch({ headless: true, channel: 'msedge' });
const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
try {
  await page.route(/\/src\/lib\/api\.ts(?:\?.*)?$/, route => route.fulfill({
    contentType: 'application/javascript',
    body: `export const nativeAvailable = true;
      export const errorMessage = String;
      export const api = {
        bootstrap: async () => ({conversations:[], config:{}, preferences:{}, runtime:{phase:'stopped',message:'',modelPath:null}, providers:[], preferredModel:{providerId:null,modelId:''}, rememberedTools:{sources:['__workspace'],tools:[{connectorId:'offline',toolName:'search'}]}}),
        hasDaytonaKey: async () => false, listConnectors: async () => [],
        listSkills: async () => [], getWorkspace: async () => ({path:'C:/Users/Papan Ghosh/Desktop/Projects/LocalLM'}),
        saveRememberedTools: async () => {}
      };`,
  }));
  await page.goto('http://localhost:1420');
  await page.getByLabel('Message', {exact:true}).fill('Keep this draft while I set up tools');
  await expect(page.getByRole('checkbox', {name:'Workspace files'})).toHaveCount(0);
  await page.getByRole('button', {name:'Tools',exact:true}).click();
  await expect(page.getByRole('heading', {name:'Tools',exact:true})).toBeVisible();
  await expect(page.getByText('Tools · 6 selected · 1 unavailable')).toBeVisible();
  await page.getByRole('button', {name:'Remove offline · search'}).click();
  await expect(page.getByText('Tools · 5 selected')).toBeVisible();
  await page.getByRole('checkbox', {name:'Workspace files'}).uncheck();
  await expect(page.getByText('Tools · Off')).toBeVisible();
  await page.getByRole('checkbox', {name:'Workspace files'}).check();
  mkdirSync('.vitest', {recursive:true});
  await page.screenshot({path:'.vitest/tools-section-desktop.png'});
  await page.setViewportSize({width:800,height:900});
  await expect(page.getByRole('checkbox', {name:'Workspace files'})).toBeVisible();
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
  expect(overflow).toBe(false);
  await page.screenshot({path:'.vitest/tools-section-narrow.png'});
  await page.getByRole('button', {name:'Back to chat'}).click();
  await expect(page.getByLabel('Message', {exact:true})).toHaveValue('Keep this draft while I set up tools');
  await expect(page.getByRole('checkbox', {name:'Workspace files'})).toHaveCount(0);
  expect(errors).toEqual([]);
  console.log('Tools section browser check passed: sidebar navigation, removal, toggles, draft preservation, desktop/narrow layout; mock backend.');
} finally {
  await browser.close();
}
