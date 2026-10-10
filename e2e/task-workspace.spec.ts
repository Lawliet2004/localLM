import { expect, test } from '@playwright/test';

// Mocked isolated-task review flow: the composer offers "Isolate task", and
// the Review tab shows the task's changes with conflict resolution, apply,
// checkpoints, and discard.
test.describe('task workspace review', () => {
  test('isolate a task, review its changes, resolve a conflict, apply and discard', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/src/lib/api.ts*', route => route.fulfill({
      contentType: 'application/javascript',
      body: `
        import { defaultRuntimeConfig } from '/src/lib/types.ts';
        export const nativeAvailable = true;
        export const errorMessage = error => String(error);
        const conversation = { id: 'c-task', title: 'Task chat', createdAt: 1, updatedAt: 2 };
        const task = { id: 't1', conversationId: 'c-task', sourceRoot: 'C:/repo', kind: 'git-worktree',
          isolatedRoot: 'C:/app/task-workspaces/t1', branch: 'localllm/task-aa11bb22', baselineCommit: 'abc123',
          status: 'active', exclusions: [{ path: '.env', reason: 'looks like a secret; never copied into task workspaces' }],
          createdAt: 1, updatedAt: 1 };
        const review = { taskId: 't1', sourceRoot: 'C:/repo', isolatedRoot: task.isolatedRoot,
          kind: 'git-worktree', branch: task.branch, baselineCommit: 'abc123', exclusions: task.exclusions,
          files: [
            { path: 'src/app.ts', oldPath: null, status: 'modified', binary: false, size: 120,
              baselineHash: 'a', taskHash: 'b', destinationHash: 'a', destinationChanged: false, conflict: false },
            { path: 'src/lib.ts', oldPath: null, status: 'modified', binary: false, size: 60,
              baselineHash: 'a', taskHash: 'b', destinationHash: 'c', destinationChanged: true, conflict: true },
            { path: 'notes.md', oldPath: null, status: 'added', binary: false, size: 5,
              baselineHash: null, taskHash: 'b', destinationHash: null, destinationChanged: false, conflict: false },
          ],
          checkpoints: [{ commit: 'deadbeef', label: 'checkpoint: before run_code', createdAt: 2, uncommitted: false }],
          applies: [] };
        let isolated = null;
        let discarded = false;
        const calls = { applied: [], discarded: 0 };
        window.__taskCalls = calls;
        export const api = {
          bootstrap: async () => ({ conversations: [conversation], config: defaultRuntimeConfig,
            preferences: { runtimePath: '', modelPath: '', temperature: 1, topP: 0.95, maxTokens: 2048, systemPrompt: '' },
            runtime: { phase: 'stopped', message: 'No model loaded', modelPath: null }, providers: [],
            preferredModel: { providerId: null, modelId: '' },
            rememberedTools: { sources: [], tools: [], accessMode: 'ask' } }),
          workspaceIndex: async () => ({ projects: [], tasks: {} }),
          getWorkspace: async () => ({ path: 'C:/repo' }), setWorkspace: async () => {},
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
            { id: 'm1', conversationId: 'c-task', role: 'user', content: 'Refactor the app', reasoning: '', status: 'complete', createdAt: 1 },
            { id: 'm2', conversationId: 'c-task', role: 'assistant', content: 'Done inside the task.', reasoning: '', status: 'complete', createdAt: 2 },
          ],
          conversationTools: async () => ({ sources: [], tools: [], accessMode: 'ask' }),
          getPreset: async () => null, getConversationRun: async () => null,
          contextPreflight: async () => ({ available: false }),
          effectiveCapabilities: async () => [],
        isolationGet: async () => ({ mode: 'unsandboxed', network: false, lan: false, lpac: false, toolchainDirs: [], processLimit: 32, memoryMb: 2048 }),
          getTodos: async () => [], getSessionEvents: async () => [],
          sendMessage: async () => {}, cancelGeneration: async () => {},
          saveConversationTools: async () => {}, saveRememberedTools: async () => {},
          createConversation: async () => 'c-new',
          saveTaskMeta: async (id, meta) => ({ projects: [], tasks: { [id]: meta } }),
          searchSessions: async () => [],
          taskWorkspaceStatus: async () => discarded ? null : isolated,
                  readiness: async () => ({step:'ready',message:'',nextAction:null}),
        qualificationResults: async () => null,
        profiles: async () => [],
        tasksList: async () => [],
          verificationSummary: async () => ({ criteria: [], records: [], state: { state: 'work-completed', summary: '' } }),
          taskWorkspaceCreate: async () => { isolated = task; return task; },
          taskReview: async () => review,
          taskFileDiff: async () => ({ text: 'diff --git a/src/app.ts\\n+changed', truncated: false, binary: false, reason: null }),
          taskCheckpoints: async () => review.checkpoints,
          taskRestoreCheckpoint: async () => {},
          taskApply: async (taskId, paths, resolutions) => {
            calls.applied.push({ paths, resolutions });
            return { applied: ['src/app.ts'], keptDestination: [], skippedConflicts: [], backupDir: 'C:/recovery/1', applyId: 'a1' };
          },
          taskUndoApply: async () => ({ restored: [], conflicts: [] }),
          taskDiscard: async () => { calls.discarded++; discarded = true; },
        };
      `,
    }));
    await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({
      contentType: 'application/javascript',
      body: 'export const getCurrentWindow = () => ({isMaximized:async()=>false,onResized:async()=>()=>{}});',
    }));
    await page.goto('/');
    // Pick the existing conversation, then isolate it from the composer.
    await page.getByRole('button', { name: 'Task chat' }).click();
    const isolate = page.getByRole('button', { name: 'Isolate task' });
    await expect(isolate).toBeVisible();
    await isolate.click();
    await expect(isolate).toHaveCount(0);
    await expect(page.getByText(/Isolated · task-aa11bb22/)).toBeVisible();

    // Review tab: change summary, file list, conflict marked and resolvable.
    await page.getByRole('button', { name: 'Review changes' }).first().click();
    await expect(page.locator('.task-review-badge')).toContainText('Isolated task');
    await expect(page.locator('.task-review-summary')).toContainText('3 changes');
    await expect(page.getByText('src/lib.ts')).toBeVisible();
    await expect(page.locator('.task-conflict-chip')).toContainText('conflict');
    await expect(page.getByText(/not carried into the task/)).toBeVisible();

    // File diff opens inline.
    await page.locator('.task-file-name', { hasText: 'src/app.ts' }).click();
    await expect(page.getByText('+changed')).toBeVisible();

    // Apply all is held back by the unresolved conflict.
    const applyAll = page.getByRole('button', { name: /Apply all/ });
    await expect(applyAll).toBeDisabled();
    await page.getByRole('combobox', { name: 'Resolve conflict for src/lib.ts' }).selectOption('take-task');
    await expect(applyAll).toBeEnabled();
    await applyAll.click();
    await expect(page.getByText(/1 file applied/)).toBeVisible();
    expect(await page.evaluate(() => (window as never as { __taskCalls: { applied: unknown[] } }).__taskCalls.applied.length)).toBe(1);

    // Checkpoints offer a restore action.
    await expect(page.getByText('checkpoint: before run_code')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Restore' })).toBeVisible();

    // Discard asks first, then removes the workspace and the composer chip.
    await page.getByRole('button', { name: /Discard task/ }).click();
    await page.getByRole('button', { name: 'Discard workspace' }).click();
    await expect(page.getByText('Task workspace discarded.')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Isolate task' })).toBeVisible();

    expect(errors).toEqual([]);
  });
});
