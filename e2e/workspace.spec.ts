import { expect, test } from '@playwright/test';

test('workspace renders, switches theme and browses catalogs without console errors', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await expect(page.getByRole('heading', { name: /Start a local conversation/ })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Think it through' })).toBeInViewport();
  await expect(page.getByRole('button', { name: 'Send message' })).toBeDisabled();
  await page.getByRole('button', { name: 'Think it through' }).click();
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue(/Help me think/);
  await page.screenshot({ path: 'test-results/workspace-dark.png', fullPage: true });
  await page.locator('summary').getByText('View', { exact: true }).click();
  await page.getByRole('button', { name: 'Switch to light theme' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.screenshot({ path: 'test-results/workspace-light.png', fullPage: true });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const rail = page.getByRole('navigation', { name: 'Settings sections' });
  await rail.getByRole('button', { name: 'Connectors' }).click();
  await page.getByRole('searchbox', { name: 'Search connectors' }).fill('Linear');
  await expect(page.locator('.catalog-list .catalog-item')).toHaveCount(1);
  await expect(page.locator('.catalog-list .catalog-item')).toContainText('Linear');
  await rail.getByRole('button', { name: 'Skills' }).click();
  await page.getByRole('searchbox', { name: 'Search skills' }).fill('notebook');
  await expect(page.locator('.catalog-list .catalog-item')).toHaveCount(1);
  await page.getByRole('searchbox', { name: 'Search settings' }).fill('temperature');
  await expect(rail.getByRole('button')).toHaveText(['Back to app', 'Generation']);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('heading', { name: /Start a local conversation/ })).toBeVisible();
  expect(errors).toEqual([]);
});

test('search shortcut opens and focuses the search field', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: /Start a local conversation/ })).toBeVisible();
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('searchbox', { name: 'Search conversations' })).toBeFocused();
});

test('Edit menu opens conversation search', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: /Start a local conversation/ })).toBeVisible();
  await page.locator('summary').getByText('Edit', { exact: true }).click();
  await page.getByRole('button', { name: /Search tasks/ }).click();
  await expect(page.getByRole('searchbox', { name: 'Search conversations' })).toBeFocused();
});

test('project header stays above its chats', async ({ page }) => {
  await page.goto('/');
  await expect(page.getByRole('heading', { name: /Start a local conversation/ })).toBeVisible();
  await page.evaluate(() => {
    const list = document.querySelector('.codex-project-list');
    if (!list) throw new Error('Project list not found');
    list.innerHTML = '<div class="codex-project-group"><div class="codex-folder-header"><button class="codex-folder-label"><svg width="13" height="13"></svg><svg width="14" height="14"></svg><span class="codex-folder-name">agent task at localmind</span></button><span class="codex-folder-count">24</span></div><div class="codex-nested-tasks"><div class="sidebar-task codex-chat-item"><button class="codex-chat-btn">First chat</button></div></div></div>';
  });
  const positions = await page.evaluate(() => {
    const header = document.querySelector('.codex-folder-header')!.getBoundingClientRect();
    const label = document.querySelector('.codex-folder-label')!.getBoundingClientRect();
    const name = document.querySelector('.codex-folder-name')!.getBoundingClientRect();
    const chat = document.querySelector('.codex-chat-item')!.getBoundingClientRect();
    return { headerBottom: header.bottom, labelBottom: label.bottom, nameBottom: name.bottom, chatTop: chat.top };
  });
  expect(positions.labelBottom).toBeLessThanOrEqual(positions.headerBottom);
  expect(positions.nameBottom).toBeLessThanOrEqual(positions.headerBottom);
  expect(positions.chatTop).toBeGreaterThanOrEqual(positions.headerBottom);
  await page.screenshot({ path: 'test-results/project-folder-layout.png' });
});

test('chat transcript keeps final output visible and work details collapsible', async ({ page }) => {
  await page.setViewportSize({ width: 1520, height: 840 });
  await page.goto('/');
  await page.evaluate(async () => {
    // @ts-expect-error vite serves this root-absolute path in the browser; tsc cannot resolve it.
    const { mountChatPreview } = await import('/e2e/chat-preview.tsx');
    mountChatPreview();
  });
  await expect(page.getByText('Worked for 3m 51s')).toBeVisible();
  await expect(page.getByText(/The transcript now keeps progress/)).toBeVisible();
  await expect(page.getByText('Edited 1 file')).toBeVisible();
  await page.screenshot({ path: 'test-results/chat-transcript-preview.png' });
  await page.getByText('Worked for 3m 51s').click();
  await expect(page.getByText('Thought', { exact: true })).toBeVisible();
  await page.getByText('Thought', { exact: true }).click();
  await expect(page.getByText('I inspected the transcript and grouped work by turn.')).toBeVisible();
  for (const width of [390, 768, 1280]) {
    await page.setViewportSize({ width, height: 840 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(page.getByText(/The transcript now keeps progress/)).toBeVisible();
  }
});

test('trajectory viewer floats clear of the composer on desktop', async ({ page }) => {
  await page.setViewportSize({ width: 1520, height: 840 });
  await page.goto('/');
  await expect(page.locator('.workspace > .trajectory')).toHaveCSS('position', 'absolute');
  await page.locator('.trajectory-summary').click();
  await expect(page.locator('.trajectory-body')).toBeVisible();
  await page.setViewportSize({ width: 390, height: 840 });
  await expect(page.locator('.workspace > .trajectory')).toHaveCSS('position', 'static');
});

test('right inspector docks, resizes, collapses, and opens Browser in the side panel', async ({ page }) => {
  await page.setViewportSize({ width: 1520, height: 840 });
  await page.goto('/');
  expect(await page.getByRole('button', { name: 'Toggle terminal' }).count()).toBe(0);
  await page.getByRole('button', { name: 'Toggle workspace panel' }).click();
  const panel = page.getByRole('complementary', { name: 'Workspace inspector' });
  await expect(panel).toBeVisible();
  await expect.poll(async () => {
    const panelBox = await panel.boundingBox();
    const chatBox = await page.locator('.chat-layout').boundingBox();
    return panelBox && chatBox ? Math.abs(chatBox.x + chatBox.width - panelBox.x) : Infinity;
  }).toBeLessThan(2);
  const before = await panel.boundingBox();
  const chat = await page.locator('.chat-layout').boundingBox();
  const resize = page.getByRole('separator', { name: 'Resize inspector' });
  const handle = await resize.boundingBox();
  if (!handle || !before) throw new Error('Inspector resize handle is missing');
  await page.mouse.move(handle.x + handle.width / 2, handle.y + 120);
  await page.mouse.down();
  await page.mouse.move(handle.x - 90, handle.y + 120, { steps: 5 });
  await page.mouse.up();
  await expect.poll(async () => (await panel.boundingBox())?.width ?? 0).toBeGreaterThan(before.width + 70);
  await page.getByRole('navigation', { name: 'Workspace shortcuts' }).getByRole('button', { name: /^Browser/ }).click();
  await expect(page.getByRole('heading', { name: 'Open a website' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Open terminal in panel' })).toHaveCount(0);
  await page.screenshot({ path: 'test-results/right-inspector-browser.png' });
  await page.getByRole('button', { name: 'Close inspector' }).click();
  await expect(panel).not.toBeVisible();
  await expect.poll(async () => (await page.locator('.chat-layout').boundingBox())?.width ?? 0).toBeGreaterThan(chat!.width + 70);
  await page.reload();
  await page.getByRole('button', { name: 'Toggle workspace panel' }).click();
  await expect.poll(async () => (await page.getByRole('complementary', { name: 'Workspace inspector' }).boundingBox())?.width ?? 0).toBeGreaterThan(before.width + 70);
  await page.setViewportSize({ width: 390, height: 840 });
  await expect.poll(async () => (await page.locator('.chat-layout').boundingBox())?.width ?? 0).toBeGreaterThan(380);
  await expect(page.getByRole('complementary', { name: 'Workspace inspector' })).toBeVisible();
});

test('runtime controls fit narrow windows and preview cannot claim to save settings', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: /Open Models/ }).click();
  await expect(page.getByRole('tab', { name: 'Discover' })).toBeVisible();
  await expect(page.getByLabel('Search Hugging Face')).toBeVisible();
  for (const width of [320, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 840 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    if (width === 768) await page.screenshot({ path: 'test-results/models-narrow.png' });
  }
  await page.setViewportSize({ width: 1280, height: 840 });
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Runtime', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Save configuration' })).toBeDisabled();
  for (const width of [320, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 840 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    if (width === 768) await page.screenshot({ path: 'test-results/runtime-narrow.png' });
  }
});

test('every settings section renders and can return to chat', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const rail = page.getByRole('navigation', { name: 'Settings sections' });
  for (const label of ['General', 'Appearance', 'Keyboard shortcuts', 'Models', 'Runtime', 'Generation',
    'Model files', 'Diagnostics', 'Tools', 'Connectors', 'Skills', 'Execution', 'Web search', 'Browser', 'Data & archive']) {
    await rail.getByRole('button', { name: label, exact: true }).click();
    await expect(page.getByRole('heading', { name: label, exact: true, level: 1 })).toBeVisible();
  }
  await page.keyboard.press('Escape');
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});
