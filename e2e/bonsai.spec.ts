import { expect, test } from '@playwright/test';

test('Bonsai remains hidden until experimental opt-in and persists across reload', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/src/lib/api.ts*', route => route.fulfill({
    contentType: 'application/javascript',
    body: `
      import { defaultRuntimeConfig } from '/src/lib/types.ts';
      export const nativeAvailable = true;
      export const errorMessage = String;
      const enabled = () => localStorage.getItem('test-bonsai-enabled') === 'true';
      const idle = {busy:false,phase:'idle',received:0,total:0,path:null,error:null};
      export const api = {
        bootstrap: async () => ({conversations:[],config:defaultRuntimeConfig,
          preferences:{runtimePath:'server.exe',modelPath:'MiniCPM5-2B.Q6_K.gguf',temperature:1,topP:0.95,maxTokens:2048,systemPrompt:''},
          runtime:{phase:'stopped',message:'No model loaded',modelPath:null},providers:[],
          preferredModel:{providerId:null,modelId:''},rememberedTools:{sources:[],tools:[],accessMode:'ask'}}),
        workspaceIndex: async () => ({projects:[],tasks:{}}),
        getWorkspace: async () => ({path:''}), setWorkspace: async () => {},
        listConnectors: async () => [], listSkills: async () => [], listCapabilities: async () => [],
        listInstalledModels: async () => [], modelInstallStatus: async () => idle,
        hardwareStatus: async () => ({gpus:[],memoryTotalBytes:17179869184,memoryAvailableBytes:4294967296}),
        searchHuggingFace: async () => [], hasDaytonaKey: async () => false,
        browserCloseAll: async () => {},
        bonsaiEnabled: async () => enabled(),
        setBonsaiEnabled: async value => {localStorage.setItem('test-bonsai-enabled',String(value));return value;},
        bonsaiInstallStatus: async () => idle,
        bonsaiInfo: async () => enabled() ? {
          catalog:{label:'Bonsai-27B',experimental:true,chatOnly:false},
          model:{filename:'Bonsai-27B-Q1_0.gguf',bytes:3803452480,sha256:'17ef842e47450caeb8eaa3ebfbbab5d2f2278b62b79be107985fb69a2f819aa0',
            destination:'C:/app/experimental/bonsai-27b/models/Bonsai-27B-Q1_0.gguf',destinationExists:false,availableBytes:70000000000,requiredBytes:4071887936},
          runtime:{bytes:704414760,requiredBytes:312971133+391443627+724707328+574138880+268435456,availableBytes:70000000000,destination:'C:/app/experimental/bonsai-27b/runtimes'},
          runtimePath:null,preset:null
        } : null,
      };
    `,
  }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({
    contentType: 'application/javascript',
    body: 'export const getCurrentWindow = () => ({isMaximized:async()=>false,onResized:async()=>()=>{}});',
  }));
  async function openModels() {
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Models', exact: true }).click();
  }
  await page.goto('/');
  await openModels();
  const toggle = page.getByRole('checkbox', { name: 'Experimental models' });
  await expect(toggle).toBeEnabled();
  await expect(toggle).not.toBeChecked();
  await expect(page.getByRole('region', { name: 'Bonsai experimental model' })).toHaveCount(0);
  await expect(page.getByText(/Bonsai-27B/)).toHaveCount(0);
  await toggle.focus();
  await page.keyboard.press('Space');
  await expect(toggle).toBeChecked();
  await expect(page.getByText('Experimental', { exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: /Bonsai-27B/ })).toBeVisible();
  await expect(page.getByText(/1-bit quality tradeoffs/)).toBeVisible();
  await page.screenshot({ path: 'test-results/bonsai-opt-in.png', fullPage: true });
  await page.reload();
  await openModels();
  await expect(toggle).toBeChecked();
  await expect(page.getByText('Experimental', { exact: true })).toBeVisible();
  await toggle.uncheck();
  await expect(page.getByRole('region', { name: 'Bonsai experimental model' })).toHaveCount(0);
  expect(errors).toEqual([]);
});
