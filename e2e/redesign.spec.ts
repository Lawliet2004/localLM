import { expect, test } from '@playwright/test';

test('home controls fit small windows and theme previews preserve the draft', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await page.evaluate(() => document.fonts.ready);
  await expect(page.getByRole('button', { name: 'Write some code', exact: true })).toBeHidden();
  await expect(page.locator('.composer-meta-bar')).toHaveCount(0);
  await expect(page.locator('.trajectory')).toHaveCount(0);
  await page.getByText('Suggestions', { exact: true }).click();
  await page.getByRole('button', { name: 'Write some code', exact: true }).click();
  await expect(page.locator('.home-starters')).not.toHaveAttribute('open');
  const message = page.getByRole('textbox', { name: 'Message', exact: true });
  await expect(message).toHaveValue(/Help me write/);
  const draft = await message.inputValue();
  for (const width of [1440, 768, 390, 320]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(message).toBeVisible();
    expect(await page.locator('.chat-layout').evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
    const bounds = await message.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
    if (width === 1440 || width === 390) {
      if (width === 390) await expect(page.locator('.sidebar')).toHaveCount(0);
      await page.screenshot({ path: `.impeccable/review/${width === 1440 ? 'desktop' : 'mobile'}.png`, animations: 'disabled' });
    }
  }
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Settings: Appearance', exact: true }).click();
  await page.getByRole('button', { name: 'Use dark appearance', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.getByRole('button', { name: 'Use dark appearance' })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('combobox', { name: 'Theme', exact: true })).toHaveValue('dark');
  await page.screenshot({ path: '.impeccable/review/settings-dark.png', animations: 'disabled' });
  await page.getByRole('button', { name: 'Conversations', exact: true }).click();
  await expect(message).toHaveValue(draft);
  await page.screenshot({ path: '.impeccable/review/home-dark.png', animations: 'disabled' });
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Settings: Appearance', exact: true }).click();
  await page.getByRole('button', { name: 'Use light appearance', exact: true }).click();
  await page.screenshot({ path: '.impeccable/review/settings-desktop.png', animations: 'disabled' });
  await page.getByRole('button', { name: 'Settings: General', exact: true }).click();
  await page.locator('.settings-page').evaluate(element => element.scrollTo({ top: element.scrollHeight, behavior: 'instant' }));
  await expect(page.locator('.general-about')).toBeInViewport();
  await page.screenshot({ path: '.impeccable/review/settings-details.png', animations: 'disabled' });
  await page.locator('.settings-page').evaluate(element => element.scrollTo({ top: 0, behavior: 'instant' }));
  await page.setViewportSize({ width: 390, height: 900 });
  await page.screenshot({ path: '.impeccable/review/settings-mobile.png', animations: 'disabled' });
  await page.getByRole('button', { name: 'Close settings', exact: true }).click();
  await expect(message).toHaveValue(draft);
  await page.reload();
  await expect(message).toHaveValue(draft);
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  expect(errors).toEqual([]);
});
