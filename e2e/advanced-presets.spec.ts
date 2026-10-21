import { expect, test } from '@playwright/test';

test('advanced profiles persist through the app save path and fit desktop and narrow settings', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(() => localStorage.setItem('locallm-appearance', JSON.stringify({ theme: 'system' })));
  // A local persistence fixture verifies the real App getPreset/setPreset
  // binding. Native SQLite reopen and preset assembly have Rust coverage.
  await page.route('**/src/lib/api.ts*', route => route.fulfill({
    contentType: 'application/javascript',
    body: `
      import { defaultRuntimeConfig } from '/src/lib/types.ts';
      export const nativeAvailable = true;
      export const errorMessage = String;
      export const api = {
        bootstrap: async () => ({ conversations:[{id:'chat',title:'Preset fixture',updatedAt:0}], config:defaultRuntimeConfig,
          preferences:{runtimePath:'',modelPath:'',temperature:1,topP:0.95,maxTokens:2048,systemPrompt:''},
          runtime:{phase:'stopped',message:'',modelPath:null}, providers:[], preferredModel:{providerId:null,modelId:''},
          rememberedTools:{sources:[],tools:[],accessMode:'ask'} }),
        messages: async () => [{id:'message',conversationId:'chat',role:'user',content:'Profile selection fixture',reasoning:'',status:'complete',createdAt:0}],
        conversationTools: async () => ({sources:[],tools:[],accessMode:'ask'}),
        getPreset: async () => ({id:localStorage.getItem('fixture-preset') || 'standard'}),
        setPreset: async (conversationId,preset) => { if(conversationId !== 'chat') throw new Error('Unexpected conversation'); localStorage.setItem('fixture-preset',preset); },
        workspaceIndex: async () => ({projects:[],tasks:{}}), setWorkspace: async () => {}, getWorkspace: async () => ({path:''}),
        listConnectors: async () => [], listSkills: async () => [], listInstalledModels: async () => [], listCapabilities: async () => [],
        modelInstallStatus: async () => ({busy:false,phase:'idle'}), hasDaytonaKey: async () => false,
        compactionStatus: async () => ({auto:true,keepLast:10,checkpoint:null})
      };`,
  }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({
    contentType: 'application/javascript',
    body: 'export const getCurrentWindow = () => ({isMaximized:async()=>false,onResized:async()=>()=>{}});',
  }));
  async function openTools() {
    await page.getByRole('button', { name: 'Preset fixture', exact: true }).click();
    await expect(page.getByText('Profile selection fixture', { exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Tools', exact: true }).click();
    await expect(page.getByRole('radiogroup', { name: 'Tool preset' })).toBeVisible();
  }
  await page.goto('/');
  await openTools();
  for (const [id, name] of [['code', /^Code \(PTC\)/], ['minimal', /^Minimal/], ['creator', /^Creator/]] as const) {
    await page.getByRole('radio', { name }).check();
    await expect.poll(() => page.evaluate(() => localStorage.getItem('fixture-preset'))).toBe(id);
    await page.reload();
    await openTools();
    await expect(page.getByRole('radio', { name })).toBeChecked();
  }
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 900 });
    for (const colorScheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme });
      await expect(page.locator('html')).toHaveAttribute('data-theme', colorScheme);
      const presets = page.getByRole('radiogroup', { name: 'Tool preset' });
      await presets.scrollIntoViewIfNeeded();
      for (const radio of await presets.getByRole('radio').all()) {
        const bounds = await radio.boundingBox();
        expect(bounds).not.toBeNull();
        expect(bounds!.x).toBeGreaterThanOrEqual(0);
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await presets.locator('..').screenshot({ path: `.impeccable/review/inference-tools/presets-${width}-${colorScheme}.png` });
    }
  }
  expect(errors).toEqual([]);
});
