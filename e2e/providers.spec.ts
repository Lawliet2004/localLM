import { expect, test } from '@playwright/test';

test('API providers tab shows the empty state and an explorable editor in the browser preview', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Settings: Models & runtime', exact: true }).click();
  await page.getByRole('tab', { name: 'API providers', exact: true }).click();

  await expect(page.getByText('No API providers yet')).toBeVisible();
  await page.getByRole('button', { name: 'Add provider', exact: true }).click();

  const editor = page.locator('.provider-editor');
  await expect(editor.getByLabel('Provider name')).toBeFocused();
  await editor.getByLabel('Quick setup').selectOption('openrouter');
  await expect(editor.getByLabel('API base URL')).toHaveValue('https://openrouter.ai/api/v1');
  await editor.getByLabel('API key', { exact: true }).fill('router-draft-key');
  await editor.getByLabel('Quick setup').selectOption('groq');
  await expect(editor.getByLabel('API base URL')).toHaveValue('https://api.groq.com/openai/v1');
  await expect(editor.getByLabel('API key', { exact: true })).toHaveValue('');
  await editor.getByLabel('Quick setup').selectOption('custom');
  await editor.getByLabel('Provider name').fill('Acme');
  await editor.getByLabel('API base URL').fill('https://api.example.com/v1');
  await editor.getByRole('textbox', { name: 'API key' }).fill('sk-preview');
  await expect(editor.getByLabel('API format')).toHaveValue('openai-chat-completions');
  await editor.getByLabel('API format').selectOption('claude-messages');
  await expect(page.getByText(/Claude endpoints always need an API key/)).toBeVisible();
  await editor.getByRole('button', { name: 'Add model ID', exact: true }).click();
  await editor.getByLabel('Model 1 ID').fill('acme-1');
  await editor.getByLabel('Model 1 context tokens').fill('8192');

  // The preview explains the boundary instead of faking a native save.
  await expect(page.getByText('Browser preview — open the desktop app to save and test providers.')).toBeVisible();
  await expect(editor.getByRole('button', { name: 'Save & test connection' })).toBeDisabled();
  await expect(editor.getByRole('button', { name: 'Save provider' })).toBeDisabled();

  // Cancelling returns to the empty state without claiming a save.
  await editor.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByText('No API providers yet')).toBeVisible();
  expect(errors).toEqual([]);
});

test('API providers tab fits narrow and desktop widths in both themes', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Settings: Models & runtime', exact: true }).click();
  await page.getByRole('tab', { name: 'API providers', exact: true }).click();
  await page.getByRole('button', { name: 'Add provider', exact: true }).click();

  for (const width of [320, 390, 768, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(page.locator('.provider-editor')).toBeVisible();
    expect(await page.locator('.settings-page').evaluate(element => element.scrollWidth <= element.clientWidth + 1)).toBe(true);
    if (width === 390 || width === 1440) {
      await page.mouse.move(0, 0);
      await page.screenshot({ path: `test-results/providers-${width}.png` });
    }
  }
  // Theme select lives in Appearance settings; switch to dark, then return.
  await page.getByRole('button', { name: 'Settings: Appearance', exact: true }).click();
  await page.getByRole('combobox', { name: 'Theme', exact: true }).selectOption('dark');
  await page.getByRole('button', { name: 'Settings: Models & runtime', exact: true }).click();
  await page.getByRole('tab', { name: 'API providers', exact: true }).click();
  await page.getByRole('button', { name: 'Add provider', exact: true }).click();
  await expect(page.locator('.provider-editor')).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.screenshot({ path: 'test-results/providers-dark.png' });
  expect(errors).toEqual([]);
});

test('saved provider retries keep one ID and refreshed models can be selected for chat', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/src/lib/api.ts*', route => route.fulfill({
    contentType: 'application/javascript',
    body: `
      import { defaultRuntimeConfig } from '/src/lib/types.ts';
      export const nativeAvailable = true;
      export const errorMessage = error => String(error);
      let providers = [], selection = {providerId:null,modelId:''}, tests = 0;
      window.__providerSaveIds = [];
      export const api = {
        bootstrap: async () => ({conversations:[],config:defaultRuntimeConfig,
          preferences:{runtimePath:'',modelPath:'',temperature:1,topP:0.95,maxTokens:2048,systemPrompt:''},
          runtime:{phase:'stopped',message:'No model loaded',modelPath:null},providers,preferredModel:selection,
          rememberedTools:{sources:[],tools:[],accessMode:'ask'}}),
        workspaceIndex:async()=>({projects:[],tasks:{}}),setWorkspace:async()=>{},getWorkspace:async()=>({path:''}),
        listConnectors:async()=>[],listSkills:async()=>[],listInstalledModels:async()=>[],listCapabilities:async()=>[],
        searchHuggingFace:async()=>[],hardwareStatus:async()=>({logicalCpus:4,memoryTotalBytes:8589934592,
          memoryAvailableBytes:4294967296,gpus:[],gpuStatus:'Fixture',sampledAt:Date.now()}),
        modelInstallStatus:async()=>({busy:false,phase:'idle',received:0,total:0,path:null,error:null}),
        hasDaytonaKey:async()=>false,contextPreflight:async()=>({available:false}),
        saveProvider:async draft=>{
          window.__providerSaveIds.push(draft.id??null);
          const old=providers.find(provider=>provider.id===draft.id);
          const saved={id:draft.id||'provider-fixture',name:draft.name,apiFormat:draft.apiFormat,
            baseUrl:draft.baseUrl,verified:false,lastTestedAt:null,hasApiKey:!!draft.apiKey||!!old?.hasApiKey,models:draft.models};
          providers=[saved];return saved;
        },
        testProvider:async id=>{
          const provider=providers.find(provider=>provider.id===id);provider.lastTestedAt=Date.now();
          if(tests++===0){provider.verified=false;throw Error('Provider rejected the authorization or API key.');}
          provider.verified=true;
          provider.models=[...provider.models.filter(model=>model.id!=='needs-context'),
            {id:'needs-context',contextLength:null,maxOutputTokens:null,toolSupport:'unknown',supportsImages:false}];
          return {verified:true,modelListSupported:true,models:provider.models.map(model=>model.id),message:'Connection verified.'};
        },
        deleteProvider:async id=>{providers=providers.filter(provider=>provider.id!==id);},
        deleteProviderApiKey:async id=>{
          const provider=providers.find(provider=>provider.id===id);
          provider.hasApiKey=false;provider.verified=false;provider.lastTestedAt=null;
        },
        savePreferredModel:async next=>{selection=next;}
      };
    `,
  }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({
    contentType: 'application/javascript',
    body: 'export const getCurrentWindow=()=>({isMaximized:async()=>false,onResized:async()=>()=>{}});',
  }));
  await page.goto('/');
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  await page.getByRole('tab', { name: 'API providers', exact: true }).click();
  await page.getByRole('button', { name: 'Add provider', exact: true }).click();
  const editor = page.locator('.provider-editor');
  await editor.getByLabel('Provider name').fill('Acme');
  await editor.getByLabel('API base URL').fill('https://api.example.com/v1');
  await editor.getByLabel('API key', { exact: true }).fill('fixture-secret');
  await editor.getByRole('button', { name: 'Add model ID', exact: true }).click();
  await editor.getByLabel('Model 1 ID').fill('fixture-model');
  await editor.getByLabel('Model 1 context tokens').fill('8192');
  await editor.getByRole('button', { name: 'Save & test connection', exact: true }).click();
  await expect(editor.getByRole('alert')).toContainText(/saved.*test.*failed/i);
  await expect(editor.getByLabel('API key', { exact: true })).toHaveValue('');
  await editor.getByRole('button', { name: 'Save & test connection', exact: true }).click();
  await expect(editor).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as {__providerSaveIds: unknown[]}).__providerSaveIds)).toEqual([null, 'provider-fixture']);
  const card = page.getByRole('region', { name: 'Acme', exact: true });
  await expect(card.getByText('Verified', { exact: true })).toBeVisible();
  await card.getByRole('button', { name: 'Use', exact: true }).click();
  await expect(card.getByText('In use', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Conversations', exact: true }).click();
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Hello');
  await expect(page.getByRole('button', { name: 'Send message', exact: true })).toBeEnabled();
  await page.locator('.composer-model').click();
  await expect(page.getByRole('dialog', { name: 'Select model' }).getByRole('button', { name: /needs-context/ })).toBeDisabled();
  expect(await page.evaluate(() => JSON.stringify(localStorage))).not.toContain('fixture-secret');
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  await page.getByRole('tab', { name: 'API providers', exact: true }).click();
  await card.getByRole('button', { name: 'Delete API key', exact: true }).click();
  await card.getByRole('button', { name: 'Delete API key', exact: true }).click();
  await expect(card.getByText('No API key saved', { exact: false })).toBeVisible();
  await expect(card.getByText('Not verified', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Conversations', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Send message', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  await page.getByRole('tab', { name: 'API providers', exact: true }).click();
  await card.getByRole('button', { name: 'Remove', exact: true }).click();
  await card.getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(card).toHaveCount(0);
  await page.getByRole('button', { name: 'Conversations', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Send message', exact: true })).toBeDisabled();
  expect(errors).toEqual([]);
});
