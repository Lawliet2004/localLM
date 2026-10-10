import { expect, test } from '@playwright/test';

test('download jobs stay visible across sections and pause only their own install', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/src/lib/api.ts*', route => route.fulfill({
    contentType: 'application/javascript',
    body: `
      import { defaultRuntimeConfig } from '/src/lib/types.ts';
      export const nativeAvailable = true;
      export const errorMessage = String;
      const idle = {busy:false,phase:'idle',received:0,total:0,path:null,error:null};
      window.__jobs = [
        {key:'runtime:standard',label:'llama.cpp CUDA runtime',busy:true,phase:'downloading',
          received:1073741824,total:2147483648,path:null,error:null,jobId:7,
          asset:'llama-runtime-x64.zip',assetIndex:2,assetCount:2,
          assetReceived:536870912,assetTotal:1073741824,saved:268435456,attempt:0,notice:null,updatedAt:0},
        {key:'model:hf:org/net@main:net-q4.gguf',label:'net-q4.gguf',busy:true,phase:'downloading',
          received:536870912,total:1073741824,path:null,error:null,jobId:9,
          asset:'net-q4.gguf',assetIndex:1,assetCount:1,
          assetReceived:536870912,assetTotal:1073741824,saved:0,attempt:0,notice:null,updatedAt:0},
      ];
      window.__cancels = [];
      window.__infoFail = false;
      export const api = {
        bootstrap: async () => ({conversations:[],config:defaultRuntimeConfig,
          preferences:{runtimePath:'server.exe',modelPath:'MiniCPM5-2B.Q6_K.gguf',temperature:1,topP:0.95,maxTokens:2048,systemPrompt:''},
          runtime:{phase:'stopped',message:'No model loaded',modelPath:null},providers:[],
          preferredModel:{providerId:null,modelId:''},rememberedTools:{sources:[],tools:[],accessMode:'ask'}}),
        workspaceIndex: async () => ({projects:[],tasks:{}}),
        getWorkspace: async () => ({path:''}), setWorkspace: async () => {},
        listConnectors: async () => [], listSkills: async () => [], listCapabilities: async () => [],
        listInstalledModels: async () => [], modelInstallStatus: async () => idle,
        downloadJobs: async () => window.__jobs,
        hardwareStatus: async () => ({gpus:[],memoryTotalBytes:17179869184,memoryAvailableBytes:4294967296,logicalCpus:8}),
        searchHuggingFace: async () => [], huggingFaceFiles: async () => ({files:[]}),
        hasDaytonaKey: async () => false, browserCloseAll: async () => {},
        listInstalledRuntimes: async () => [],
        modelDownloadInfo: async () => { if (window.__infoFail) throw new Error('model info offline');
          return {bytes:100,requiredBytes:356,availableBytes:1024,destination:'C:/models/m.gguf',destinationExists:false,savedBytes:0,remainingBytes:100}; },
        runtimeDownloadInfo: async () => ({bytes:1,requiredBytes:2,availableBytes:10,destination:'C:/runtime',savedBytes:0,reusable:false}),
        readModelMetadata: async () => null,
        runtimeStatus: async () => ({phase:'stopped',message:'No model loaded',modelPath:null}),
        compactionStatus: async () => ({auto:true,keepLast:10,checkpoint:null}),
        messages: async () => [], conversationTools: async () => ({sources:[],tools:[],accessMode:'ask'}),
        getPreset: async () => null, workspaceGit: async () => null,
        taskWorkspaceStatus: async () => null,
                readiness: async () => ({step:'ready',message:'',nextAction:null}),
        qualificationResults: async () => null,
        profiles: async () => [],
        tasksList: async () => [],
          verificationSummary: async () => ({ criteria: [], records: [], state: { state: 'work-completed', summary: '' } }),
        cancelModelInstall: async jobId => { window.__cancels.push(['cancelModelInstall', jobId]); },
        cancelRuntimeInstall: async jobId => { window.__cancels.push(['cancelRuntimeInstall', jobId]); },
      };
    `,
  }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({
    contentType: 'application/javascript',
    body: 'export const getCurrentWindow = () => ({isMaximized:async()=>false,onResized:async()=>()=>{}});',
  }));

  const rail = () => page.getByRole('navigation', { name: 'Settings sections' });
  async function openSettings(section: string) {
    if (await page.getByRole('button', { name: 'Settings', exact: true }).isVisible().catch(() => false)) {
      await page.getByRole('button', { name: 'Settings', exact: true }).click();
    }
    await rail().getByRole('button', { name: section, exact: true }).click();
  }
  const runtime = page.getByRole('region', { name: 'llama.cpp CUDA runtime' });
  const hf = page.getByRole('region', { name: 'net-q4.gguf' });

  await page.goto('/');
  await openSettings('Models');
  await expect(runtime.getByText('Downloading archive 2 of 2')).toBeVisible();
  await expect(runtime.getByText('1.00 GiB / 2.00 GiB')).toBeVisible();
  await expect(runtime.getByText('0.25 GiB were already saved')).toBeVisible();
  await expect(hf.getByText('Downloading')).toBeVisible();
  await expect(hf.getByText('0.50 GiB / 1.00 GiB')).toBeVisible();

  // Jobs survive navigation: Runtime section, chat, and back to Models.
  await rail().getByRole('button', { name: 'Runtime', exact: true }).click();
  await expect(runtime.getByText('Downloading archive 2 of 2')).toBeVisible();
  await expect(hf.getByText('Downloading')).toBeVisible();
  await page.getByRole('button', { name: 'Back to app' }).click();
  await expect(runtime.getByText('Downloading archive 2 of 2')).toBeVisible();
  await expect(hf.getByText('Downloading')).toBeVisible();
  await openSettings('Models');
  await expect(runtime.getByRole('button', { name: 'Pause' })).toBeVisible();
  await expect(hf.getByRole('button', { name: 'Pause' })).toBeVisible();

  // Pausing the runtime row targets only its own job.
  await runtime.getByRole('button', { name: 'Pause' }).click();

  await expect.poll(() => page.evaluate(() => (window as any).__cancels)).toEqual([['cancelRuntimeInstall', 7]]);

  // Panel info failures never hide the shared status rows.
  await page.evaluate(() => { (window as any).__infoFail = true; });
  await rail().getByRole('button', { name: 'Model files', exact: true }).click();
  await expect(page.getByText('model info offline')).toBeVisible();
  await expect(runtime.getByText('Downloading archive 2 of 2')).toBeVisible();
  await expect(hf.getByText('Downloading')).toBeVisible();

  // A failed job reports its error after navigating away and back.
  await page.evaluate(() => {
    (window as any).__jobs = (window as any).__jobs.map((job: any) => job.key === 'runtime:standard'
      ? { ...job, busy: false, phase: 'failed', error: 'checksum mismatch for llama-runtime-x64.zip' }
      : job);
  });
  await page.getByRole('button', { name: 'Back to app' }).click();
  await expect(runtime.getByRole('alert')).toHaveText('checksum mismatch for llama-runtime-x64.zip');
  await openSettings('Models');
  await expect(runtime.getByText('Failed')).toBeVisible();
  await expect(runtime.getByRole('alert')).toHaveText('checksum mismatch for llama-runtime-x64.zip');
  await expect(hf.getByText('Downloading')).toBeVisible();
  expect(errors).toEqual([]);
});
