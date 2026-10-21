import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

async function setup(page: Page, options = {}) {
  await page.addInitScript(value => { (window as any).__auditOptions = value; }, options);
  await page.route('**/src/lib/api.ts*', route => route.fulfill({ contentType: 'application/javascript', body: readFileSync('e2e/fixtures/audit-api.js', 'utf8') }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const getCurrentWindow=()=>({isMaximized:async()=>false,onResized:async()=>()=>{}})' }));
  await page.route(/.*@tauri-apps_plugin-dialog\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const open=async()=>"/fixture/picked";export const save=async()=>null;export const confirm=async()=>true;' }));
  await page.goto('/');
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeEnabled();
}

test('missing project preserves draft and attachment', async ({ page }) => {
  await setup(page, { noProjects: true });
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await input.fill('Important draft');
  await page.locator('input[type=file]').setInputFiles({ name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('Important attachment') });
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(input).toHaveValue('Important draft');
  await expect(page.getByRole('button', { name: /Remove notes.txt/ })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__audit.sends)).toBe(0);
});

test('saved prompt is not restored as a duplicate draft after inference fails', async ({ page }) => {
  await setup(page);
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await input.fill('Saved prompt');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByText('Fixture inference failure after prompt saved', { exact: true })).toBeVisible();
  await expect(input).toHaveValue('');
  expect(await page.evaluate(() => (window as any).__audit.sends)).toBe(1);
});

test('Stop during conversation creation prevents later backend send and restores input', async ({ page }) => {
  await setup(page, { prepareDelay: 700 });
  const input = page.getByRole('textbox', { name: 'Message', exact: true });
  await input.fill('Do not send after Stop');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await page.getByRole('button', { name: 'Stop response' }).click();
  await expect(input).toHaveValue('Do not send after Stop');
  expect(await page.evaluate(() => (window as any).__audit.sends)).toBe(0);
});

test('Ctrl+N follows current project and resolves the first folder from All projects', async ({ page }) => {
  await setup(page);
  await expect(page.getByRole('button', { name: 'New conversation', exact: true })).toBeEnabled();
  await page.keyboard.press('Control+n');
  await expect.poll(() => page.evaluate(() => (window as any).__audit.workspaces.slice(-1)[0])).toBe('/fixture/a');
  await page.getByRole('button', { name: 'Project B', exact: true }).focus();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('button', { name: 'New chat in Project B', exact: true })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByLabel('Task project', { exact: true })).toHaveValue('b');
  await page.keyboard.press('Control+n');
  await expect(page.getByLabel('Task project', { exact: true })).toHaveValue('b');
  await expect.poll(() => page.evaluate(() => (window as any).__audit.workspaces.slice(-1)[0])).toBe('/fixture/b');
});

test('pause failure allows another attempt and Resume appears only after checkpoint settlement', async ({ page }) => {
  await setup(page, { hold: true, pauseFailure: true });
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Keep working');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__audit.sends)).toBe(1);
  await page.getByRole('button', { name: 'Pause response' }).click();
  await expect(page.getByText('Pause unavailable', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Pause response' })).toBeEnabled();
  await expect(page.getByRole('button', { name: 'Resume response' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Stop response' }).click();
});

test('paused checkpoint is scoped to its conversation', async ({ page }) => {
  await setup(page, { hold: true });
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Pause this task');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__audit.sends)).toBe(1);
  await page.getByRole('button', { name: 'Pause response' }).click();
  await expect(page.getByRole('button', { name: 'Resume response' })).toBeVisible();
  await page.keyboard.press('Control+n');
  await expect(page.getByRole('button', { name: 'Resume response' })).toHaveCount(0);
});

test('folder picker updates task scope used by next send', async ({ page }) => {
  await setup(page);
  await page.getByRole('button', { name: 'Tools', exact: true }).click();
  await page.getByRole('button', { name: /Choose folder|Change folder/ }).click();
  await page.getByRole('button', { name: 'Back to chat' }).click();
  await expect(page.getByLabel('Task project', { exact: true })).toHaveValue('picked');
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Use the selected folder');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__audit.sentWorkspace)).toBe('/fixture/picked');
});

test('trajectory pages beyond 1000 events in the real browser', async ({ page }) => {
  await setup(page, { trajectory: true });
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Open trajectory');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const trajectory = page.locator('.trajectory');
  await trajectory.locator('summary').first().click();
  await expect(trajectory.getByText('#500', { exact: true })).toBeVisible();
  await trajectory.getByRole('button', { name: /Load more events/ }).click();
  await expect(trajectory.getByText('#1000', { exact: true })).toBeVisible();
  await trajectory.getByRole('button', { name: /Load more events/ }).click();
  await expect(trajectory.getByText('#1200', { exact: true })).toBeVisible();
  await expect(trajectory.locator('.trajectory-event')).toHaveCount(1200);
  await expect(trajectory.getByText('All 1200 events loaded.')).toBeVisible();
  await expect(trajectory.getByRole('button', { name: /Load more events/ })).toHaveCount(0);
});

test('trajectory refresh discards a delayed older page', async ({ page }) => {
  await setup(page, { trajectory: true, delayTrajectoryPage: true });
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Open trajectory');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  const trajectory = page.locator('.trajectory');
  await trajectory.locator('summary').first().click();
  await expect(trajectory.getByText('#500', { exact: true })).toBeVisible();
  await trajectory.getByRole('button', { name: /Load more events/ }).click();
  await trajectory.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(trajectory.getByText('#500', { exact: true })).toBeVisible();
  await page.evaluate(() => (window as any).__audit.releasePages());
  await expect(trajectory.getByText('#501', { exact: true })).toHaveCount(0);
  await expect(trajectory.locator('.trajectory-event')).toHaveCount(500);
});
