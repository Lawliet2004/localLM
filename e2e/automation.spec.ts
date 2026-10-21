import { expect, test } from '@playwright/test';

test('browser and computer settings have separate destinations and native actions stay disabled in preview', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Keep this draft across automation settings.');
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Settings: Execution', exact: true }).click();
  await expect(page.getByLabel('Enable browser use')).toHaveCount(0);
  await expect(page.getByLabel('Enable computer use')).toHaveCount(0);

  for (const [name, mode] of [['Browser Use', 'browser'], ['Computer Use', 'computer']]) {
    const tab = page.getByRole('button', { name: `Settings: ${name}`, exact: true });
    await tab.click();
    await expect(tab).toHaveAttribute('aria-current', 'page');
    await expect(page.getByRole('heading', { name, exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: `Save ${mode} settings` })).toBeDisabled();
    await expect(page.getByRole('button', { name: `Check ${mode} setup` })).toBeDisabled();
    await expect(page.getByLabel(`Enable ${mode} use`)).toBeDisabled();
    await expect(page.getByText('Setup and automation require the LocalLM desktop app.')).toBeVisible();
    if (mode === 'browser') {
      await expect(page.getByRole('button', { name: 'Set up browser engine' })).toBeDisabled();
      await expect(page.getByLabel('Enable computer use')).toHaveCount(0);
    } else {
      await expect(page.getByLabel('Enable browser use')).toHaveCount(0);
      await expect(page.getByRole('button', { name: 'Reset browser sessions' })).toHaveCount(0);
    }
    for (const width of [320, 390, 768, 1440]) {
      await page.setViewportSize({ width, height: 1000 });
      await page.locator('.automation-settings').evaluate(el => el.scrollTo({ top: 0, behavior: 'instant' }));
      await expect(page.getByRole('heading', { name, exact: true })).toBeVisible();
      const tabBounds = await tab.boundingBox();
      expect(tabBounds!.x).toBeGreaterThanOrEqual(0);
      expect(tabBounds!.x + tabBounds!.width).toBeLessThanOrEqual(width);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      expect(await page.locator('.automation-settings').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
      if (mode === 'browser') {
        const paths = page.getByText('Browser paths', { exact: true });
        await paths.click();
        await expect(page.getByLabel('Browser engine package folder')).toBeVisible();
        expect(await page.locator('.automation-settings').evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
        await paths.click();
        await page.locator('.automation-settings').evaluate(el => el.scrollTo({ top: 0, behavior: 'instant' }));
      }
      if (width === 390 || width === 1440) {
        await page.mouse.move(0, 0);
        await page.screenshot({ path: `.impeccable/review/${mode}-use-${width}.png`, animations: 'disabled' });
        await page.getByRole('button', { name: 'Use dark theme', exact: true }).click();
        await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
        await page.mouse.move(0, 0);
        await page.screenshot({ path: `.impeccable/review/${mode}-use-${width}-dark.png`, animations: 'disabled' });
        await page.getByRole('button', { name: 'Use light theme', exact: true }).click();
      }
    }
  }
  await page.getByRole('button', { name: 'Settings: Browser Use', exact: true }).click();
  await page.getByRole('button', { name: 'Configure Node.js', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Execution', exact: true })).toBeVisible();
  const search = page.getByRole('searchbox', { name: 'Search settings' });
  await search.fill('playwright');
  await expect(page.getByRole('button', { name: 'Settings: Browser Use', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Settings: Computer Use', exact: true })).toHaveCount(0);
  await search.fill('screenshots');
  await expect(page.getByRole('button', { name: 'Settings: Computer Use', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Close settings', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('Keep this draft across automation settings.');
  expect(errors).toEqual([]);
});
