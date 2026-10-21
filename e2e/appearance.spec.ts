import { expect, test, type Page } from '@playwright/test';

async function openAppearance(page: Page) {
  await page.getByRole('button', { name: 'Open settings', exact: true }).click();
  await page.getByRole('button', { name: 'Settings: Appearance', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Appearance', exact: true })).toBeVisible();
}

test('appearance applies to the workspace, persists across reload, and resets independently of drafts', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Keep this draft.');
  await openAppearance(page);
  await page.getByRole('button', { name: 'Use dark appearance', exact: true }).click();
  await page.getByRole('button', { name: 'Violet accent', exact: true }).click();
  await page.getByLabel('Conversation font', { exact: true }).selectOption('serif');
  expect(await page.evaluate(async () => (await document.fonts.load('14px "IBM Plex Serif"')).length)).toBeGreaterThan(0);
  const size = page.getByRole('slider', { name: 'Text size', exact: true });
  await size.focus();
  await page.keyboard.press('End');
  await expect(size).toHaveValue('20');
  await page.getByLabel('Code size', { exact: true }).selectOption('18');
  await page.getByLabel('Interface density', { exact: true }).selectOption('compact');
  await page.getByLabel('Conversation width', { exact: true }).selectOption('wide');
  await page.getByLabel('Animations', { exact: true }).selectOption('reduced');
  await expect(page.locator('.appearance-reading-preview p')).toHaveCSS('font-size', '20px');
  await expect(page.locator('.appearance-reading-preview code')).toHaveCSS('font-size', '18px');
  await expect(page.locator('.preference-row').first()).toHaveCSS('min-height', '65px');
  await page.getByRole('button', { name: 'Close settings', exact: true }).click();
  const composer = page.getByRole('textbox', { name: 'Message', exact: true });
  await expect(composer).toHaveValue('Keep this draft.');
  await expect(composer).toHaveCSS('font-size', '20px');
  await expect(composer).toHaveCSS('font-family', /IBM Plex Serif/);
  await expect(page.locator('.composer-region')).toHaveCSS('max-width', '1080px');
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.locator('html')).toHaveAttribute('data-accent', 'violet');
  await expect(composer).toHaveValue('Keep this draft.');
  await expect(composer).toHaveCSS('font-size', '20px');
  await openAppearance(page);
  await expect(page.getByLabel('Code size', { exact: true })).toHaveValue('18');
  await expect(page.getByLabel('Interface density', { exact: true })).toHaveValue('compact');
  await expect(page.getByLabel('Animations', { exact: true })).toHaveValue('reduced');
  await page.getByRole('button', { name: 'Reset appearance', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Reset appearance', exact: true })).toBeDisabled();
  await expect(page.getByLabel('Theme', { exact: true })).toHaveValue('light');
  await expect(page.locator('html')).toHaveAttribute('data-accent', 'forest');
  await expect(page.locator('html')).toHaveAttribute('data-motion', 'system');
  await page.getByRole('button', { name: 'Close settings', exact: true }).click();
  await expect(composer).toHaveValue('Keep this draft.');
  await expect(composer).toHaveCSS('font-size', '14px');
  expect(errors).toEqual([]);
});

test('system theme follows the OS while a manual choice stays fixed', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/');
  await openAppearance(page);
  await page.getByRole('button', { name: 'Use system appearance', exact: true }).click();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await expect(page.getByLabel('Theme', { exact: true })).toHaveValue('system');
  await expect(page.getByText('Follows your system. Currently using dark appearance.')).toBeVisible();
  await page.reload();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
  await openAppearance(page);
  await expect(page.getByLabel('Theme', { exact: true })).toHaveValue('system');
  await page.getByRole('button', { name: 'Use light appearance', exact: true }).click();
  await page.emulateMedia({ colorScheme: 'light' });
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await page.getByRole('button', { name: 'Close settings', exact: true }).click();
  await page.getByRole('button', { name: 'Use dark theme', exact: true }).click();
  await openAppearance(page);
  await expect(page.getByLabel('Theme', { exact: true })).toHaveValue('dark');
});

test('all appearance controls fit narrow windows, large text, and both themes', async ({ page }) => {
  await page.goto('/');
  await openAppearance(page);
  for (const width of [320, 390, 768, 1024, 1440]) {
    await page.setViewportSize({ width, height: 1000 });
    for (const theme of ['light', 'dark']) {
      await page.getByLabel('Theme', { exact: true }).selectOption(theme);
      await page.locator('.settings-page').evaluate(element => element.scrollTo({ top: 0, behavior: 'instant' }));
      expect(await page.locator('.settings-page').evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
      for (const control of await page.locator('.appearance-settings select, .appearance-settings input').all()) {
        const bounds = await control.boundingBox();
        expect(bounds!.x).toBeGreaterThanOrEqual(0);
        expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width);
      }
      if (width === 1440 || width === 390) {
        await page.evaluate(() => document.fonts.ready);
        await page.mouse.move(0, 0);
        await page.screenshot({ path: `.impeccable/review/appearance-${width === 1440 ? 'desktop' : 'mobile'}-${theme}.png`, animations: 'disabled' });
        if (width === 1440 && theme === 'light') {
          await page.locator('.settings-page').evaluate(element => element.scrollTo({ top: element.scrollHeight, behavior: 'instant' }));
          await page.screenshot({ path: '.impeccable/review/appearance-details.png', animations: 'disabled' });
        }
      }
    }
  }
  await page.setViewportSize({ width: 320, height: 840 });
  await page.getByRole('slider', { name: 'Text size', exact: true }).focus();
  await page.keyboard.press('End');
  await page.getByLabel('Code size', { exact: true }).selectOption('18');
  expect(await page.locator('.settings-page').evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true);
  await page.getByRole('button', { name: 'Reset appearance', exact: true }).click();
});

test('reduced motion honors both the system and the explicit preference', async ({ page }) => {
  await page.goto('/');
  await openAppearance(page);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(page.getByRole('button', { name: 'Use dark appearance', exact: true })).toHaveCSS('transition-duration', '0s');
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  await page.getByLabel('Animations', { exact: true }).selectOption('reduced');
  await expect(page.getByRole('button', { name: 'Use dark appearance', exact: true })).toHaveCSS('transition-duration', '0s');
  await expect(page.locator('.settings-page')).toHaveCSS('scroll-behavior', 'auto');
});
