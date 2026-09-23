import { expect, test } from '@playwright/test';

test('reference-style chrome opens workspace tabs and keeps them usable', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/');
  await expect(page.locator('.app-menubar')).toBeVisible();
  await page.getByRole('button', { name: 'Toggle workspace panel' }).click();
  await expect(page.getByRole('navigation', { name: 'Workspace shortcuts' })).toBeVisible();
  // The surface covers the main column; the menubar sidebar toggle still works.
  await page.getByRole('button', { name: 'Toggle sidebar', exact: true }).click();
  await page.getByRole('button', { name: 'Toggle sidebar', exact: true }).click();
  await page.screenshot({ path: 'test-results/reference-chrome.png' });
  await page.getByRole('button', { name: 'Review Ctrl+Shift+G' }).click();
  await expect(page.getByText('Choose a workspace to review its changes.')).toBeVisible();
  // The + menu launches additional workspace tabs.
  await page.locator('.workspace-add summary').click();
  await page.getByRole('button', { name: 'Files Ctrl+P' }).click();
  await expect(page.getByText('Choose a workspace folder to browse files.')).toBeVisible();
  await page.keyboard.press('Control+t');
  await expect(page.getByRole('heading', { name: 'Open a website' })).toBeVisible();
  await page.locator('.workspace-tab-actions').getByRole('button', { name: 'Expand panel' }).click();
  await expect(page.locator('.workspace-panel')).toHaveClass(/panel-expanded/);
  await page.getByRole('button', { name: 'Close inspector' }).click();
  await expect(page.getByRole('complementary', { name: 'Workspace inspector' })).toHaveCount(0);
  for (const width of [320, 768, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  expect(errors).toEqual([]);
});
