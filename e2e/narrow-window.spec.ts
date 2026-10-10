import { expect, test } from '@playwright/test';

// Minimum supported window (760×560): the sidebar leaves the grid and opens
// as a dismissible overlay so the composer and primary actions stay reachable.
test.describe('narrow window', () => {
  test.use({ viewport: { width: 760, height: 560 } });

  test('sidebar opens as a dismissible overlay over the conversation', async ({ page }) => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.route('**/src/lib/api.ts*', route => route.fulfill({
      contentType: 'application/javascript',
      body: `
        import { defaultRuntimeConfig } from '/src/lib/types.ts';
        export const nativeAvailable = true;
        export const errorMessage = error => String(error);
        const conversation = { id: 'c-narrow', title: 'Existing chat', createdAt: 1, updatedAt: 2 };
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
            { id: 'm1', conversationId: 'c-narrow', role: 'user', content: 'Hi', reasoning: '', status: 'complete', createdAt: 1 },
            { id: 'm2', conversationId: 'c-narrow', role: 'assistant', content: 'Hello!', reasoning: '', status: 'complete', createdAt: 2 },
          ],
          conversationTools: async () => ({ sources: [], tools: [], accessMode: 'ask' }),
          getPreset: async () => null, getConversationRun: async () => null,
          contextPreflight: async () => ({ available: false }),
          effectiveCapabilities: async () => [],
        isolationGet: async () => ({ mode: 'unsandboxed', network: false, lan: false, lpac: false, toolchainDirs: [], processLimit: 32, memoryMb: 2048 }),
          taskWorkspaceStatus: async () => null,
                  readiness: async () => ({step:'ready',message:'',nextAction:null}),
        qualificationResults: async () => null,
        profiles: async () => [],
        tasksList: async () => [],
          verificationSummary: async () => ({ criteria: [], records: [], state: { state: 'work-completed', summary: '' } }),
          getTodos: async () => [], getSessionEvents: async () => [],
          sendMessage: async () => {}, cancelGeneration: async () => {},
          saveConversationTools: async () => {}, saveRememberedTools: async () => {},
          createConversation: async () => 'c-new',
          saveTaskMeta: async (id, meta) => ({ projects: [], tasks: { [id]: meta } }),
          searchSessions: async () => [],
        };
      `,
    }));
    await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({
      contentType: 'application/javascript',
      body: 'export const getCurrentWindow = () => ({isMaximized:async()=>false,onResized:async()=>()=>{}});',
    }));

    await page.goto('/');

    // Welcome: the sidebar stays closed by default at this width and the
    // composer and primary actions are fully reachable.
    const toggle = page.getByRole('button', { name: 'Toggle sidebar' });
    const composer = page.getByRole('textbox', { name: 'Message', exact: true });
    await expect(page.getByRole('heading', { name: /Start a local conversation/ })).toBeVisible();
    await expect(page.locator('#app-sidebar')).toHaveCount(0);
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await expect(toggle).toHaveAttribute('aria-controls', 'app-sidebar');
    await expect(composer).toBeInViewport();
    await expect(page.getByRole('button', { name: 'Send message' })).toBeAttached();

    // Menubar toggle opens the overlay and moves focus into it.
    await toggle.click();
    const overlay = page.getByRole('dialog', { name: 'Conversations' });
    await expect(overlay).toBeVisible();
    await expect(toggle).toHaveAttribute('aria-expanded', 'true');
    await expect(overlay).toBeFocused();

    // Selecting a conversation closes the overlay; the chat fills the window.
    await overlay.getByRole('button', { name: 'Existing chat' }).click();
    await expect(page.locator('#app-sidebar')).toHaveCount(0);
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await expect(page.getByText('Hello!')).toBeVisible();
    await expect(composer).toBeInViewport();
    // Typing reveals the send button even at the minimum window size.
    await composer.fill('Hi');
    await expect(page.getByRole('button', { name: 'Send message' })).toBeVisible();
    await composer.fill('');

    // Reopen, then Escape closes it and returns focus to the toggle.
    await toggle.click();
    await expect(overlay).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('#app-sidebar')).toHaveCount(0);
    await expect(toggle).toBeFocused();

    // Scrim click closes too.
    await toggle.click();
    await expect(overlay).toBeVisible();
    await page.locator('.sidebar-scrim').click({ position: { x: 700, y: 300 } });
    await expect(page.locator('#app-sidebar')).toHaveCount(0);
    await expect(toggle).toBeFocused();

    // The inspector panel still fits at this width.
    await page.getByRole('button', { name: 'Toggle workspace panel' }).click();
    await expect(page.getByRole('complementary', { name: 'Workspace inspector' })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.getByRole('button', { name: 'Close inspector' }).click();

    // Settings live in the sidebar — at this width they are reached through
    // the overlay, which closes as settings take over the window.
    await toggle.click();
    await overlay.getByRole('button', { name: 'Settings', exact: true }).click();
    const rail = page.getByRole('navigation', { name: 'Settings sections' });
    await expect(rail.getByRole('button', { name: 'General', exact: true })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'General', exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.keyboard.press('Escape');
    await expect(page.getByText('Hello!')).toBeVisible();
    await expect(composer).toBeVisible();

    // Widening restores the normal sidebar column (user's collapsed state).
    await page.setViewportSize({ width: 1280, height: 800 });
    await expect(page.locator('#app-sidebar')).toBeVisible();
    await expect(page.locator('.sidebar-scrim')).toHaveCount(0);

    expect(errors).toEqual([]);
  });
});
