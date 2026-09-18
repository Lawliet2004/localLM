import { expect, test } from '@playwright/test';

test('workspace renders, switches theme and browses catalogs without console errors', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await expect(page.getByRole('heading', { name: /A little model/ })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Send message' })).toBeDisabled();
  await page.getByRole('button', { name: 'Think it through' }).click();
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue(/Help me think/);
  await page.screenshot({ path: 'test-results/workspace-dark.png', fullPage: true });
  await page.getByRole('button', { name: 'Use light theme' }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.screenshot({ path: 'test-results/workspace-light.png', fullPage: true });
  await page.getByRole('button', { name: 'Connectors', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Search connectors' }).fill('Linear');
  await expect(page.locator('.catalog-list .catalog-item')).toHaveCount(1);
  await expect(page.locator('.catalog-list .catalog-item')).toContainText('Linear');
  await page.getByRole('button', { name: 'Skills', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Search skills' }).fill('notebook');
  await expect(page.locator('.catalog-list .catalog-item')).toHaveCount(1);
  expect(errors).toEqual([]);
});

test('runtime controls fit narrow windows and preview cannot claim to save settings', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: /Open Models/ }).click();
  await expect(page.getByRole('tab', { name: 'Discover' })).toBeVisible();
  await expect(page.getByLabel('Search Hugging Face')).toBeVisible();
  for (const width of [320, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 840 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  await page.setViewportSize({ width: 1280, height: 840 });
  await page.getByRole('tab', { name: 'Runtime', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Save configuration' })).toBeDisabled();
  for (const width of [320, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 840 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
});
