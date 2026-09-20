import { expect, test } from '@playwright/test';

test('permission choice survives a new chat and page restart', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  // Native persistence is tested with a reopened SQLite Store in Rust. This
  // fixture exercises the browser controls and bootstrap contract on reload.
  await page.route('**/src/lib/api.ts*', route => route.fulfill({
    contentType: 'application/javascript',
    body: `
      import { defaultRuntimeConfig } from '/src/lib/types.ts';
      export const nativeAvailable = true;
      export const errorMessage = error => String(error);
      const read = () => JSON.parse(localStorage.getItem('test-remembered-tools') || '{"sources":[],"tools":[],"accessMode":"ask"}');
      export const api = {
        bootstrap: async () => ({ conversations: [], config: defaultRuntimeConfig,
          preferences: {runtimePath:'',modelPath:'',temperature:1,topP:0.95,maxTokens:2048,systemPrompt:''},
          runtime:{phase:'stopped',message:'',modelPath:null}, providers:[],
          preferredModel:{providerId:null,modelId:''}, rememberedTools:read() }),
        saveRememberedTools: async tools => localStorage.setItem('test-remembered-tools', JSON.stringify(tools)),
        workspaceIndex: async () => ({projects:[],tasks:{}}),
        setWorkspace: async () => {}, getWorkspace: async () => ({path:''}),
        listConnectors: async () => [{id:'deepwiki',description:'Documentation',url:'https://example.invalid/mcp',authType:'none',connected:false,hasCredential:false,tools:[],connectionError:'Service unavailable.'}], listSkills: async () => [],
        listInstalledModels: async () => [], listCapabilities: async () => [],
        modelInstallStatus: async () => ({busy:false,phase:'idle'}),
        hasDaytonaKey: async () => false
      };
    `,
  }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({
    contentType: 'application/javascript',
    body: 'export const getCurrentWindow = () => ({isMaximized:async()=>false,onResized:async()=>()=>{}});',
  }));
  await page.goto('/');
  for (const mode of ['Full access', 'Auto-approve reads', 'Ask for approval']) {
    await page.getByRole('button', { name: /^Permission mode:/ }).click();
    await page.getByRole('dialog', { name: 'Select permission mode' }).getByRole('button', { name: new RegExp(mode) }).click();
    await expect(page.getByRole('button', { name: `Permission mode: ${mode}`, exact: true })).toBeEnabled();
    await page.keyboard.press('Control+n');
    await expect(page.getByRole('button', { name: `Permission mode: ${mode}`, exact: true })).toBeVisible();
    await page.reload();
    await expect(page.getByRole('button', { name: `Permission mode: ${mode}`, exact: true })).toBeEnabled();
  }
  await page.screenshot({ path: 'test-results/session-settings.png' });
  await page.getByRole('button', { name: 'Connectors', exact: true }).click();
  await page.locator('.catalog-item summary').filter({ hasText: 'Deepwiki' }).click();
  await expect(page.getByRole('alert')).toContainText('Could not reconnect: Service unavailable.');
  await expect(page.getByRole('button', { name: 'Disconnect', exact: true })).toBeEnabled();
  expect(errors).toEqual([]);
});
