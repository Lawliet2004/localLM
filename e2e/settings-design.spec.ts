import { expect, test } from '@playwright/test';

test('native tool settings fit desktop and narrow windows in both themes', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  for (const theme of ['light', 'dark']) {
    await page.getByRole('button', { name: 'Settings: Appearance', exact: true }).click();
    await page.getByRole('combobox', { name: 'Theme', exact: true }).selectOption(theme);
    for (const width of [1280, 390]) {
      await page.setViewportSize({ width, height: 900 });
      for (const [destination, heading] of [['Bash', 'Bash & shell'], ['Python', 'Python'], ['Web Search', 'Web Search'], ['Weather', 'Weather'], ['Security', 'Security'], ['Git', 'Git']]) {
        await page.getByRole('button', { name: `Settings: ${destination}`, exact: true }).click();
        await expect(page.getByRole('heading', { name: heading, exact: true })).toBeVisible();
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
        const controlsFit = await page.locator('.settings-page input:not([type=checkbox]), .settings-page textarea, .settings-page select').evaluateAll(controls => controls.every(control => {
          const bounds = control.getBoundingClientRect(); return bounds.x >= 0 && bounds.right <= innerWidth;
        }));
        expect(controlsFit).toBe(true);
        if (destination === 'Python' || destination === 'Web Search') await page.screenshot({ path: `test-results/feature-settings-${destination}-${theme}-${width}.png` });
      }
    }
  }
});

test('settings search, appearance, and navigation preserve the conversation draft', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Keep this draft while I change settings.');
  await page.keyboard.press('Control+,');
  await expect(page.getByRole('heading', { name: 'General', exact: true })).toBeVisible();
  const search = page.getByRole('searchbox', { name: 'Search settings' });
  await search.fill('gpu');
  await expect(page.getByRole('button', { name: 'Settings: Models & runtime' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Settings: General' })).toHaveCount(0);
  await search.fill('no matching category');
  await expect(page.getByRole('status')).toContainText('No settings found');
  await page.getByRole('button', { name: 'Clear settings search' }).click();
  await expect(search).toBeFocused();
  await page.getByRole('button', { name: 'Settings: Appearance', exact: true }).click();
  await page.getByRole('combobox', { name: 'Theme', exact: true }).selectOption('dark');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await page.mouse.move(800, 0);
  await page.screenshot({ path: 'test-results/settings-dark.png' });
  await page.getByRole('button', { name: 'Close settings', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('Keep this draft while I change settings.');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('Keep this draft while I change settings.');
  expect(errors).toEqual([]);
});

test('all settings destinations work and the rail always returns to chat', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  for (const [name, heading] of [
    ['Models & runtime', 'Models & Runtime'], ['Tools', 'Tools'], ['Execution', 'Execution'],
    ['Browser Use', 'Browser Use'], ['Computer Use', 'Computer Use'],
    ['Bash', 'Bash & shell'], ['Python', 'Python'], ['Weather', 'Weather'], ['Web Search', 'Web Search'],
    ['Inference', 'Inference'], ['Diagnostics', 'Diagnostics'], ['Security', 'Security'], ['Git', 'Git'],
    ['Connectors', 'Connectors'], ['Skills', 'Skills'], ['Appearance', 'Appearance'], ['General', 'General'],
  ]) {
    const button = page.getByRole('button', { name: `Settings: ${name}`, exact: true });
    await button.click();
    await expect(button).toHaveAttribute('aria-current', 'page');
    await expect(page.getByRole('heading', { name: heading, exact: true }).first()).toBeVisible();
  }
  await expect(page.getByRole('radio', { name: 'Ask for approval' })).toBeDisabled();
  await page.getByRole('button', { name: 'Tools & permissions', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Tools', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Conversations', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeVisible();
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('searchbox', { name: 'Search conversations', exact: true })).toBeFocused();
  expect(errors).toEqual([]);
});

test('settings adapt to narrow windows and remain operable in both themes', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Settings: Appearance', exact: true }).click();
  for (const width of [320, 390, 768, 1024, 1440, 1920]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(page.getByRole('heading', { name: 'Appearance', exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const theme = page.getByRole('combobox', { name: 'Theme', exact: true });
    await theme.scrollIntoViewIfNeeded();
    const bounds = await theme.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
    await theme.selectOption('dark');
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await theme.selectOption('light');
    await page.locator('.settings-page').evaluate(element => element.scrollTop = 0);
    if (width === 390 || width === 1440) {
      await page.mouse.move(0, 0);
      await page.screenshot({ path: `test-results/settings-${width}.png` });
    }
  }
});

test('small windows keep settings controls inside the canvas and restore the chat sidebar state', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.setViewportSize({ width: 320, height: 840 });
  await page.goto('/');
  await expect(page.locator('.sidebar')).toHaveCount(0);
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Close settings', exact: true }).click();
  await expect(page.locator('.sidebar')).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeVisible();
  for (const width of [320, 390, 768]) {
    await page.setViewportSize({ width, height: 840 });
    for (const name of ['Models & runtime', 'Execution', 'Connectors', 'Skills', 'Tools']) {
      await page.getByRole('navigation', { name: 'Main navigation' }).getByRole('button', { name, exact: true }).click();
      expect(await page.locator('.settings-page').evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
    }
  }
  // The Edit menu uses the same search action as Ctrl+K, even from settings.
  await page.getByText('Edit', { exact: true }).click();
  await page.getByRole('button', { name: /Search tasks/ }).click();
  await expect(page.getByRole('searchbox', { name: 'Search conversations', exact: true })).toBeFocused();
  expect(errors).toEqual([]);
});
