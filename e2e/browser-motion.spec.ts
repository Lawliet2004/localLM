import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

test.beforeEach(async ({ page }) => {
  const html = (await (await page.request.get('/')).text()).replace('/src/main.tsx', '/e2e/fixtures/workspace-harness.tsx');
  await page.route('**/__workspace-test', route => route.fulfill({ contentType: 'text/html', body: html }));
  await page.route('**/src/lib/api.ts*', route => route.fulfill({ contentType: 'application/javascript', body: readFileSync('e2e/fixtures/workspace-api.js', 'utf8') }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const getCurrentWindow=()=>({onCloseRequested:async()=>()=>{},destroy:async()=>{}});' }));
  await page.route(/.*@tauri-apps_api_event\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const listen=async()=>()=>{};' }));
  await page.goto('/__workspace-test');
  await expect(page.getByRole('button', { name: 'Toggle workspace panel' })).toBeVisible();
  await page.keyboard.press('Control+t');
  await page.getByRole('textbox', { name: 'Address', exact: true }).fill('http://localhost:3000');
  await page.getByRole('button', { name: 'Go', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__workspaceFixture.browserState().views.filter((v: any) => v.visible).length)).toBe(1);
});

async function aligned(page: Page) {
  await expect.poll(() => page.evaluate(() => {
    const host = document.querySelector('.browser-host')!;
    const rect = host.getBoundingClientRect();
    const expected = [rect.left, rect.top, rect.width, rect.height].map(Math.round);
    const views = (window as any).__workspaceFixture.browserState().views.filter((v: any) => v.visible);
    return views.length === 1 && JSON.stringify(views[0].bounds) === JSON.stringify(expected);
  })).toBe(true);
}

test('native host stays aligned on reveal, expansion, position-only moves, and responsive resize', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await aligned(page);
  await page.getByRole('button', { name: 'Close inspector' }).click();
  await page.getByRole('button', { name: 'Toggle workspace panel' }).click();
  // Observe the entrance frames, not just a screenshot after the animation.
  expect(await page.locator('.workspace-panel').evaluate(async panel => {
    const frames = [];
    for (let i = 0; i < 12; i++) {
      await new Promise(requestAnimationFrame);
      const style = getComputedStyle(panel);
      frames.push(style.transform === 'none' && style.opacity === '1' && style.animationName === 'none');
    }
    return frames.every(Boolean);
  })).toBe(true);
  await aligned(page);
  await page.locator('.workspace-panel').evaluate(panel => { panel.style.left = '40px'; panel.style.right = '-40px'; });
  await aligned(page);
  await page.locator('.workspace-panel').evaluate(panel => { panel.style.left = ''; panel.style.right = ''; });
  await page.getByRole('complementary').getByRole('button', { name: 'Expand panel', exact: true }).click();
  await aligned(page);
  await page.getByRole('complementary').getByRole('button', { name: 'Restore panel size', exact: true }).click();
  await aligned(page);
  for (const width of [390, 768, 1280]) {
    await page.setViewportSize({ width, height: 840 });
    await aligned(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: `test-results/browser-motion-${width}.png` });
  }
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.getByRole('button', { name: 'Close inspector' }).click();
  await page.keyboard.press('Control+t');
  await aligned(page);
  expect(errors).toEqual([]);
});

test('hides the native surface during ancestor animation and restores its final position', async ({ page }) => {
  await page.addStyleTag({ content: '@keyframes host-motion-test { from { transform: translateX(50px); opacity: .4; } to { transform: none; opacity: 1; } }' });
  const count = await page.evaluate(() => (window as any).__workspaceFixture.browserState().calls.length);
  await page.locator('#root > div').evaluate(root => { root.style.animation = 'host-motion-test .5s linear'; });
  await expect.poll(() => page.evaluate(() => (window as any).__workspaceFixture.browserState().views.filter((v: any) => v.visible).length)).toBe(0);
  await aligned(page);
  expect(await page.evaluate(start => (window as any).__workspaceFixture.browserState().calls.slice(start).map((call: any) => call.kind), count)).toEqual(['hide', 'show']);
});

test('slow native operations coalesce and cannot reveal an old tab over its replacement', async ({ page }) => {
  await page.evaluate(() => { (window as any).__workspaceFixture.browserDelay = 180; });
  const initialShows = await page.evaluate(() => (window as any).__workspaceFixture.browserState().calls.filter((c: any) => c.kind === 'show').length);
  await page.locator('.workspace-panel').evaluate(async panel => {
    for (let i = 1; i <= 8; i++) {
      panel.style.left = `${i * 4}px`; panel.style.right = `${-i * 4}px`;
      await new Promise(requestAnimationFrame);
    }
  });
  await aligned(page);
  const shows = await page.evaluate(() => (window as any).__workspaceFixture.browserState().calls.filter((c: any) => c.kind === 'show').length);
  expect(shows - initialShows).toBeLessThanOrEqual(2);
  await page.locator('.workspace-panel').evaluate(panel => { panel.style.left = '60px'; panel.style.right = '-60px'; });
  await page.getByRole('button', { name: 'Review changes', exact: true }).last().click();
  await expect.poll(() => page.evaluate(() => (window as any).__workspaceFixture.browserState().views.filter((v: any) => v.visible).length)).toBe(0);
  await page.keyboard.press('Control+t');
  await aligned(page);
  await page.getByRole('button', { name: 'Close localhost' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__workspaceFixture.browserState().views.length)).toBe(0);
});
