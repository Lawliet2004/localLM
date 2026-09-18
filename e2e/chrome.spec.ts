import { expect, test } from '@playwright/test';

test('reference-style chrome opens workspace shortcuts and keeps panels usable', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto('/');
  await page.getByRole('button', { name: 'Collapse sidebar', exact: true }).click();
  await expect(page.locator('.app-menubar')).toBeVisible();
  await page.getByRole('button', { name: 'Toggle files panel' }).click();
  await expect(page.getByRole('navigation', { name: 'Workspace shortcuts' })).toBeVisible();
  await page.getByRole('button', { name: 'Expand sidebar', exact: true }).click();
  await page.screenshot({ path: 'test-results/reference-chrome.png' });
  await page.getByRole('button', { name: 'Review Ctrl+Shift+G' }).click();
  await expect(page.getByText('Choose a workspace to review its changes.')).toBeVisible();
  await page.getByRole('button', { name: 'Panel shortcuts', exact: true }).click();
  await page.getByRole('button', { name: 'Files Ctrl+P' }).click();
  await expect(page.getByText('Choose a workspace folder to browse files.')).toBeVisible();
  await page.keyboard.press('Control+t');
  await expect(page.getByRole('heading', { name: 'Open a website' })).toBeVisible();
  await page.getByRole('button', { name: 'Expand panel', exact: true }).click();
  await expect(page.locator('.workspace-panel')).toHaveClass(/panel-expanded/);
  await page.getByRole('button', { name: 'Close inspector' }).click();
  await expect(page.getByRole('complementary', { name: 'Workspace inspector' })).toHaveCount(0);
  for (const width of [320, 768, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  expect(errors).toEqual([]);
});
