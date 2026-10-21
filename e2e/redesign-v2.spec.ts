import { test } from '@playwright/test';
import { readFileSync } from 'fs';

const fixture = readFileSync('e2e/fixtures/audit-api.js', 'utf8') + '\n' + readFileSync('e2e/fixtures/redesign-v2-api.js', 'utf8');
const dir = '.impeccable/review/redesign-v2';

for (const theme of ['light', 'dark']) {
  for (const width of [1440, 390, 800]) {
    test(`redesign v2 ${theme} ${width}px`, async ({ page }) => {
      test.setTimeout(240000);
      await page.addInitScript(value => {
        localStorage.setItem('locallm-appearance', JSON.stringify({ theme: value, accent: 'forest', motion: 'reduced' }));
      }, theme);
      await page.route('**/src/lib/api.ts*', route => route.fulfill({ contentType: 'application/javascript', body: fixture }));
      await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const getCurrentWindow=()=>({isMaximized:async()=>false,onResized:async()=>()=>{}})' }));
      await page.route(/.*@tauri-apps_plugin-dialog\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const open=async()=>"/fixture/picked";export const save=async()=>null;export const confirm=async()=>true;' }));
      const shot = (name: string) => page.screenshot({ path: `${dir}/${name}.png`, animations: 'disabled' });

      await page.setViewportSize({ width, height: 900 });
      await page.goto('/');
      await page.evaluate(() => document.fonts.ready);
      await shot(`home-${theme}-${width}`);

      if (width < 760) await page.getByRole('button', { name: 'Toggle sidebar' }).first().click();
      await page.locator('button', { hasText: 'GenUI demo' }).first().click();
      // Wait for the GenUI renderer to mount — a source-only block would mean a fallback.
      await page.locator('.genui-block').first().getByRole('button', { name: 'Show details' }).waitFor();
      // Composer toolbar children must never overlap — at any width. The
      // check covers the visible control wrappers; hidden popovers are ignored.
      const collisions = await page.evaluate(() => {
        const els = [...document.querySelectorAll('.composer-bottom > div > *')]
          .filter(el => {
            const rect = el.getBoundingClientRect();
            const style = getComputedStyle(el);
            return rect.width > 2 && rect.height > 2 && style.display !== 'none' && style.visibility !== 'hidden' && style.pointerEvents !== 'none'
              && !el.classList.contains('sr-only'); // visually hidden controls are not collisions
          });
        const bad: string[] = [];
        for (let i = 0; i < els.length; i++) for (let j = i + 1; j < els.length; j++) {
          if (els[i].contains(els[j]) || els[j].contains(els[i])) continue;
          const a = els[i].getBoundingClientRect(), b = els[j].getBoundingClientRect();
          if (a.left < b.right - 0.5 && b.left < a.right - 0.5 && a.top < b.bottom - 0.5 && b.top < a.bottom - 0.5)
            bad.push(`${els[i].getAttribute('aria-label') || els[i].className} × ${els[j].getAttribute('aria-label') || els[j].className}`);
        }
        return bad;
      });
      if (collisions.length) throw new Error(`Composer toolbar overlap at ${width}px: ${collisions.join('; ')}`);
      await shot(`chat-genui-${theme}-${width}`);

      await page.locator('.genui-block').first().getByRole('button', { name: 'Open in canvas' }).click();
      const canvas = page.getByRole('complementary', { name: 'Canvas:' });
      await canvas.waitFor();
      await canvas.getByRole('button', { name: 'Show details' }).waitFor();
      // Docked keeps chat ≥ ~380; below that the canvas must be a ≥320px overlay.
      const bounds = await page.evaluate(() => ({
        chat: document.querySelector('.chat-layout')?.getBoundingClientRect().width ?? 0,
        canvas: document.querySelector('.canvas-panel')?.getBoundingClientRect().width ?? 0,
        overlay: !!document.querySelector('.canvas-panel')?.offsetParent && getComputedStyle(document.querySelector('.canvas-panel') as Element).position === 'absolute',
      }));
      if (width >= 760 && bounds.canvas < 320) throw new Error(`Canvas squeezed to ${bounds.canvas}px at ${width}px viewport`);
      await shot(`canvas-${theme}-${width}`);

      await page.getByRole('button', { name: 'Open settings' }).click();
      await shot(`settings-modal-${theme}-${width}`);
      await page.getByRole('button', { name: 'Settings: Models & runtime' }).click();
      await page.getByRole('tab', { name: 'API providers' }).click();
      await page.getByRole('button', { name: 'Add provider' }).first().click();
      await page.getByLabel('Quick setup').selectOption('ollama');
      await shot(`provider-form-${theme}-${width}`);
    });
  }
}
