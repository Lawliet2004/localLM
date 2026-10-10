import { expect, test } from '@playwright/test';

// The task inbox lists queued/running work with state chips, wait reasons,
// and per-task actions — all against the mocked api module.
test('task inbox shows queued and running tasks with actions', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const calls: { name: string; args: unknown }[] = [];
  await page.exposeFunction('e2eTaskCall', (name: string, args: unknown) => calls.push({ name, args }));
  await page.route('**/src/lib/api.ts*', route => route.fulfill({
    contentType: 'application/javascript',
    body: `
      import { defaultRuntimeConfig } from '/src/lib/types.ts';
      export const nativeAvailable = true;
      export const errorMessage = error => String(error);
      const conversation = { id: 'c1', title: 'Task chat', createdAt: 1, updatedAt: 2 };
      const tasks = [
        { id: 't-run', conversationId: 'c1', content: 'summarize the repo', interactionMode: 'act',
          status: 'running', position: null, runId: 'r1', waitReason: null, error: null,
          nextAction: null, parentTaskId: null, createdAt: 1, startedAt: 2, endedAt: null },
        { id: 't-queued', conversationId: 'c1', content: 'then write tests', interactionMode: 'act',
          status: 'queued', position: 2, runId: null, waitReason: null, error: null,
          nextAction: null, parentTaskId: null, createdAt: 3, startedAt: null, endedAt: null },
      ];
      export const api = {
        bootstrap: async () => ({ conversations: [conversation], config: defaultRuntimeConfig,
          preferences: { runtimePath: '', modelPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' },
          runtime: { phase: 'stopped', message: 'No model loaded', modelPath: null }, providers: [],
          preferredModel: { providerId: null, modelId: '' },
          rememberedTools: { sources: [], tools: [], accessMode: 'ask' } }),
        workspaceIndex: async () => ({ projects: [], tasks: {} }),
        getWorkspace: async () => ({ path: '' }), setWorkspace: async () => {},
        workspaceGit: async () => null, workspaceInspect: async () => null,
        listConnectors: async () => [], listSkills: async () => [], listCapabilities: async () => [],
        listInstalledModels: async () => [], listInstalledRuntimes: async () => [],
        modelInstallStatus: async () => ({ busy: false, phase: 'idle' }),
        downloadJobs: async () => [], hasDaytonaKey: async () => false,
        browserCloseAll: async () => {},
        hardwareStatus: async () => ({ gpus: [], memoryTotalBytes: 0, memoryAvailableBytes: 0, logicalCpus: 0 }),
        runtimeStatus: async () => ({ phase: 'stopped', message: 'No model loaded', modelPath: null }),
        compactionStatus: async () => ({ auto: true, keepLast: 10, checkpoint: null }),
        messages: async () => [
          { id: 'm1', conversationId: 'c1', role: 'user', content: 'Go', reasoning: '', status: 'complete', createdAt: 1 },
        ],
        conversationTools: async () => ({ sources: [], tools: [], accessMode: 'ask' }),
        getPreset: async () => null, getConversationRun: async () => null,
        contextPreflight: async () => ({ available: false }),
        effectiveCapabilities: async () => [],
        isolationGet: async () => ({ mode: 'unsandboxed', network: false, lan: false, lpac: false, toolchainDirs: [], processLimit: 32, memoryMb: 2048 }),
        taskWorkspaceStatus: async () => null,
        verificationSummary: async () => ({ criteria: [], records: [], state: { state: 'work-completed', summary: '' } }),
        getTodos: async () => [], getSessionEvents: async () => [],
        sendMessage: async () => {}, cancelGeneration: async () => {},
        saveConversationTools: async () => {}, saveRememberedTools: async () => {},
        createConversation: async () => 'c-new',
        saveTaskMeta: async (id, meta) => ({ projects: [], tasks: { [id]: meta } }),
        searchSessions: async () => [],
                readiness: async () => ({step:'ready',message:'',nextAction:null}),
        qualificationResults: async () => null,
        profiles: async () => [],
        tasksList: async () => tasks,
        taskPause: async (args) => window.e2eTaskCall('task_pause', args),
        taskResume: async (args) => window.e2eTaskCall('task_resume', args),
        taskCancel: async (args) => window.e2eTaskCall('task_cancel', args),
        taskRetry: async (args) => window.e2eTaskCall('task_retry', args),
        taskEnqueue: async (args) => window.e2eTaskCall('task_enqueue', args),
      };
    `,
  }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({
    contentType: 'application/javascript',
    body: 'export const getCurrentWindow = () => ({isMaximized:async()=>false,onResized:async()=>()=>{}});',
  }));
  await page.goto('/');
  await page.getByRole('button', { name: 'Task chat' }).click();

  // The inbox shows both tasks with status chips.
  await expect(page.locator('.task-inbox')).toBeVisible();
  await expect(page.locator('.task-inbox')).toContainText('summarize the repo');
  await expect(page.locator('.task-inbox')).toContainText('then write tests');
  await expect(page.locator('.task-chip').first()).toHaveText('Running');
  await expect(page.locator('.task-chip').nth(1)).toHaveText('Queued');

  // Pause and cancel act on the running task's id.
  await page.getByRole('button', { name: 'Pause task summarize the repo' }).click();
  await page.getByRole('button', { name: 'Cancel task summarize the repo' }).click();
  expect(calls.map(call => call.name)).toContain('task_pause');
  expect(calls.map(call => call.name)).toContain('task_cancel');

  // Clicking a task opens its conversation — already selected, stays on chat.
  await page.getByTitle('then write tests').click();
  await expect(page.locator('.task-inbox')).toBeVisible();
  expect(errors).toEqual([]);
});
