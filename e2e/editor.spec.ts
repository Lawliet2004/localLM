import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';

test.beforeEach(async ({ page }) => {
  const html = (await (await page.request.get('/')).text()).replace('/src/main.tsx', '/e2e/fixtures/workspace-harness.tsx');
  await page.route('**/__workspace-test', route => route.fulfill({ contentType: 'text/html', body: html }));
  await page.route('**/src/lib/api.ts*', route => route.fulfill({ contentType: 'application/javascript', body: readFileSync('e2e/fixtures/workspace-api.js', 'utf8') }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const getCurrentWindow=()=>({onCloseRequested:async()=>()=>{},destroy:async()=>{}});' }));
  await page.goto('/__workspace-test');
  await page.getByRole('button', { name: 'Toggle workspace panel' }).click();
  await page.locator('.workspace-empty').getByRole('button', { name: /^Editor/ }).click();
});

async function openCode(page: import('@playwright/test').Page) {
  await page.locator('.workspace-tree').getByRole('button', { name: 'src', exact: true }).click();
  await page.locator('.workspace-tree').getByRole('button', { name: 'main.ts', exact: true }).click();
  return page.getByRole('textbox', { name: 'Edit src/main.ts' });
}

test('real editor retains drafts, undo and tab state, then saves complete text with CRLF', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  const editor = await openCode(page);
  await editor.click();
  await page.keyboard.press('Control+End');
  await expect(page.locator('.editor-status')).toContainText('Ln 653');
  await page.keyboard.press('Control+Home');
  await page.keyboard.press('Control+A');
  await page.keyboard.insertText('const saved = 123;\n');
  await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible();
  await page.locator('.workspace-tree').getByRole('button', { name: 'README.md', exact: true }).click();
  await page.getByRole('tab', { name: /main.ts/ }).click();
  await expect(editor).toContainText('const saved = 123;');
  await editor.click();
  await page.keyboard.press('Control+z');
  await expect(editor).toContainText('// original');
  await page.keyboard.press('Control+Shift+z');
  await expect(editor).toContainText('const saved = 123;');
  await page.getByRole('button', { name: 'Close inspector' }).click();
  await page.getByRole('button', { name: 'Toggle workspace panel' }).click();
  await expect(editor).toContainText('const saved = 123;');
  await page.getByRole('button', { name: 'Close main.ts' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click();
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.press('Control+s');
  await expect(page.getByText('Unsaved changes', { exact: true })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__workspaceFixture.content('src/main.ts'))).toBe('const saved = 123;\r\n');
  await page.screenshot({ path: 'test-results/editor-desktop.png' });
  expect(errors).toEqual([]);
});

test('code size updates a mounted editor without losing edits or selection', async ({ page }) => {
  const editor = await openCode(page);
  await editor.click();
  await page.keyboard.press('Control+Home');
  await page.keyboard.insertText('// unsaved work\n');
  await page.evaluate(async () => {
    const { applyAppearance, defaultAppearance } = await import('/src/lib/appearance.ts' as string);
    applyAppearance({ ...defaultAppearance, codeFontSize: 18 });
  });
  await expect(editor).toHaveCSS('font-size', '18px');
  await expect(editor).toContainText('// unsaved work');
  await expect(page.getByText('Unsaved changes', { exact: true })).toBeVisible();
  await editor.click();
  await page.keyboard.press('Control+z');
  await expect(editor).not.toContainText('// unsaved work');
});

test('creates entries, renames a folder with an open document, and confirms deletion', async ({ page }) => {
  const tree = page.locator('.workspace-tree');
  await tree.getByRole('button', { name: 'New folder', exact: true }).click();
  await page.getByRole('dialog').getByLabel('Name or relative path').fill('feature');
  await page.getByRole('dialog').getByRole('button', { name: 'Create' }).click();
  await tree.getByRole('button', { name: 'feature', exact: true }).click();
  await tree.getByRole('button', { name: 'New file', exact: true }).click();
  await page.getByRole('dialog').getByLabel('Name or relative path').fill('test.ts');
  await page.getByRole('dialog').getByRole('button', { name: 'Create' }).click();
  const editor = page.getByRole('textbox', { name: 'Edit feature/test.ts' });
  await editor.click(); await page.keyboard.insertText('export const value = true;\n');
  await tree.getByRole('button', { name: 'feature', exact: true }).click();
  await tree.getByRole('button', { name: 'Rename selected' }).click();
  await page.getByRole('dialog').getByLabel('New name').fill('renamed');
  await page.getByRole('dialog').getByRole('button', { name: 'Rename', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Edit renamed/test.ts' })).toContainText('export const value = true;');
  await tree.getByRole('button', { name: 'renamed', exact: true }).click();
  await tree.getByRole('button', { name: 'Delete selected' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByRole('tab', { name: /test.ts/ })).toBeVisible();
  await tree.getByRole('button', { name: 'Delete selected' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByRole('tab', { name: /test.ts/ })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__workspaceFixture.paths())).not.toContain('renamed/test.ts');
});

test('save conflicts preserve drafts; search and responsive layout remain usable', async ({ page }) => {
  const editor = await openCode(page);
  await editor.click(); await page.keyboard.press('Control+A'); await page.keyboard.insertText('my unsaved draft');
  await page.evaluate(() => (window as any).__workspaceFixture.externalEdit('src/main.ts', 'external edit\n'));
  await page.keyboard.press('Control+s');
  await expect(page.getByRole('alert')).toContainText('This file changed on disk');
  await expect(editor).toHaveText('my unsaved draft');
  await page.getByRole('button', { name: 'Reload file', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Cancel' }).click();
  await expect(editor).toHaveText('my unsaved draft');
  await page.getByRole('button', { name: 'Reload file', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Discard & reload' }).click();
  await expect(editor).toHaveText('external edit');
  await editor.click(); await page.keyboard.press('Control+f');
  await expect(page.locator('.cm-search')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('complementary', { name: 'Workspace inspector' })).toBeVisible();
  for (const width of [390, 768, 1280]) {
    await page.setViewportSize({ width, height: 840 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(editor).toBeVisible();
    await page.screenshot({ path: `test-results/editor-${width}.png` });
  }
  await page.evaluate(() => { document.documentElement.dataset.theme = 'dark'; });
  await page.screenshot({ path: 'test-results/editor-dark.png', animations: 'disabled' });
  await page.getByLabel('Filter files').fill('README');
  await page.locator('.workspace-tree').getByRole('button', { name: 'README.md', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Edit README.md' })).toContainText('# Workspace');
});
