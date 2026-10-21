import { expect, test, type Page } from '@playwright/test';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';

// One coordinated review batch. Set REVIEW_VISUAL_ROUND=2 only after the single
// consolidated repair pass. Functional acceptance does not take screenshots.
const round = process.env.REVIEW_VISUAL_ROUND || '1';
const directory = `.impeccable/review/review-round-${round}`;
test.use({ screenshot: 'off', reducedMotion: 'reduce' });

async function setup(page: Page, theme: string) {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(theme => {
    localStorage.setItem('locallm-appearance', JSON.stringify({ theme, motion: 'reduced' }));
  }, theme);
  await page.route('**/src/lib/api.ts*', route => route.fulfill({ contentType: 'application/javascript', body: readFileSync('e2e/fixtures/audit-api.js', 'utf8') + '\n' + readFileSync('e2e/fixtures/review-api.js', 'utf8') }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const getCurrentWindow=()=>({isMaximized:async()=>false,onResized:async()=>()=>{}})' }));
  await page.route(/.*@tauri-apps_plugin-dialog\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const open=async()=>null;export const save=async()=>null;export const confirm=async()=>true;' }));
  await page.route('https://assets.example.test/chart.svg', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="#356349"/></svg>' }));
  await page.goto('/');
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeEnabled();
  if ((page.viewportSize()?.width ?? 1440) < 760) await page.getByRole('button', { name: 'Expand sidebar', exact: true }).click();
  await page.getByRole('button', { name: 'Review acceptance conversation', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Answer ready', exact: true })).toBeVisible();
  if ((page.viewportSize()?.width ?? 1440) < 760) await page.getByRole('button', { name: 'Collapse sidebar', exact: true }).click();
  await page.evaluate(() => document.fonts.ready);
  return errors;
}

async function observations(page: Page) {
  return page.evaluate(() => {
    const canvas = document.createElement('canvas'); canvas.width = canvas.height = 1;
    const ctx = canvas.getContext('2d')!;
    const cache = new Map<string, number[]>();
    function rgba(color: string) {
      if (!cache.has(color)) { ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = color; ctx.fillRect(0, 0, 1, 1); const value = [...ctx.getImageData(0, 0, 1, 1).data]; cache.set(color, [value[0], value[1], value[2], value[3] / 255]); }
      return cache.get(color)!;
    }
    function over(fg: number[], bg: number[]) {
      const alpha = fg[3] + bg[3] * (1 - fg[3]);
      return alpha ? [0, 1, 2].map(i => (fg[i] * fg[3] + bg[i] * bg[3] * (1 - fg[3])) / alpha).concat(alpha) : [0, 0, 0, 0];
    }
    function luminance(rgb: number[]) { return rgb.slice(0, 3).map(value => { value /= 255; return value <= .04045 ? value / 12.92 : ((value + .055) / 1.055) ** 2.4; }).reduce((sum, value, i) => sum + value * [.2126, .7152, .0722][i], 0); }
    const contrast: { text: string; selector: string; ratio: number; minimum: number; foreground: string; background: number[] }[] = [];
    const overflow: { selector: string; width: number; scrollWidth: number }[] = [];
    for (const element of document.querySelectorAll<HTMLElement>('body *')) {
      const style = getComputedStyle(element), bounds = element.getBoundingClientRect();
      if (!bounds.width || !bounds.height || bounds.bottom <= 0 || bounds.top >= innerHeight || bounds.right <= 0 || bounds.left >= innerWidth || style.visibility === 'hidden' || style.display === 'none' || style.opacity === '0') continue;
      const selector = element.tagName.toLowerCase() + (element.classList.length ? '.' + [...element.classList].join('.') : '');
      if (element.matches('.chat-scroll,.composer-region,.settings-page,.artifact-viewer,.command-palette,.latency-diagnostic') && element.scrollWidth > element.clientWidth + 2) overflow.push({ selector, width: element.clientWidth, scrollWidth: element.scrollWidth });
      if (!Array.from(element.childNodes).some(node => node.nodeType === Node.TEXT_NODE && node.textContent?.trim()) || element.closest(':disabled,[aria-disabled="true"]') || bounds.width < 3 || bounds.height < 3) continue;
      let background = [0, 0, 0, 0];
      for (let current: HTMLElement | null = element; current && background[3] < .999; current = current.parentElement) background = over(background, rgba(getComputedStyle(current).backgroundColor));
      background = over(background, [255, 255, 255, 1]);
      const foreground = over(rgba(style.color), background);
      const a = luminance(foreground), b = luminance(background);
      const ratio = (Math.max(a, b) + .05) / (Math.min(a, b) + .05);
      const minimum = parseFloat(style.fontSize) >= 24 || parseFloat(style.fontSize) >= 18.66 && parseInt(style.fontWeight) >= 700 ? 3 : 4.5;
      if (ratio + .02 < minimum) contrast.push({ text: element.innerText?.trim().slice(0, 90) || '', selector, ratio: Number(ratio.toFixed(2)), minimum, foreground: style.color, background: background.slice(0, 3).map(Math.round) });
    }
    return { viewport: { width: innerWidth, height: innerHeight }, documentOverflow: document.documentElement.scrollWidth > innerWidth + 2, overflow,
      contrastCandidates: contrast.sort((a, b) => a.ratio - b.ratio).slice(0, 40),
      reducedMotion: matchMedia('(prefers-reduced-motion: reduce)').matches,
      runningAnimations: document.getAnimations().filter(animation => animation.playState === 'running').length };
  });
}

for (const viewport of [{ width: 1440, height: 1000, name: 'desktop' }, { width: 390, height: 844, name: 'narrow' }]) {
  for (const theme of ['light', 'dark']) {
    test(`expanded tool ${viewport.name} ${theme}`, async ({ page }) => {
      mkdirSync(directory, { recursive: true });
      await page.setViewportSize(viewport);
      const errors = await setup(page, theme);
      await page.locator('.activity-summary > summary').click();
      const web = page.getByTestId('command-run-card').filter({ hasText: 'Searched the web' });
      await web.locator(':scope > summary').click();
      await web.scrollIntoViewIfNeeded();
      await page.screenshot({ path: `${directory}/${viewport.name}-${theme}-tool-result.png`, animations: 'disabled' });
      writeFileSync(`${directory}/${viewport.name}-${theme}-tool-observations.json`, JSON.stringify({ errors, scene: await observations(page) }, null, 2));
      expect(errors).toEqual([]);
    });
    test(`visual batch ${viewport.name} ${theme}`, async ({ page }) => {
      test.setTimeout(90000);
      mkdirSync(directory, { recursive: true });
      await page.setViewportSize(viewport);
      const errors = await setup(page, theme);
      const scenes: Record<string, unknown> = {};
      async function capture(name: string) {
        await page.mouse.move(0, 0);
        await page.screenshot({ path: `${directory}/${viewport.name}-${theme}-${name}.png`, animations: 'disabled' });
        scenes[name] = await observations(page);
      }
      await page.locator('.chat-scroll').evaluate(element => { element.scrollTop = 0; });
      await capture('conversation');
      await page.locator('.activity-summary > summary').click();
      await page.getByRole('button', { name: 'Preview exports/result-00.csv', exact: true }).click();
      await page.getByRole('region', { name: 'Artifact exports/result-00.csv', exact: true }).scrollIntoViewIfNeeded();
      await capture('artifact');
      await page.getByRole('button', { name: 'Open settings', exact: true }).click();
      await page.getByRole('heading', { name: 'Project documents', exact: true }).scrollIntoViewIfNeeded();
      await capture('project-documents');
      await page.getByRole('button', { name: 'Settings: Inference', exact: true }).click();
      await expect(page.getByRole('heading', { name: 'Inference', exact: true })).toBeVisible();
      await capture('generation');
      await page.getByRole('button', { name: 'Settings: Diagnostics', exact: true }).click();
      await expect(page.getByRole('heading', { name: 'Response latency', exact: true })).toBeVisible();
      await capture('diagnostics');
      await page.getByLabel('Load diagnostic report', { exact: true }).setInputFiles('.impeccable/review/response-diagnostic-quick-cpu/measurements.json');
      await page.getByRole('region', { name: 'Latency measurements', exact: true }).scrollIntoViewIfNeeded();
      await capture('diagnostic-results');
      if (viewport.name === 'desktop') {
        await page.getByRole('button', { name: 'Close settings', exact: true }).click();
        scenes.paletteChecks = await page.evaluate(async theme => {
          const modulePath = '/src/lib/appearance.ts';
          const appearance = await import(/* @vite-ignore */ modulePath);
          const canvas = document.createElement('canvas'); canvas.width = canvas.height = 1;
          const context = canvas.getContext('2d')!;
          function luminance(color: string) {
            context.clearRect(0, 0, 1, 1); context.fillStyle = color; context.fillRect(0, 0, 1, 1);
            return [...context.getImageData(0, 0, 1, 1).data].slice(0, 3).map(v => { v /= 255; return v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4; }).reduce((sum, v, i) => sum + v * [.2126, .7152, .0722][i], 0);
          }
          const results = Object.keys(appearance.accentColors).map(accent => {
            appearance.applyAppearance({ ...appearance.defaultAppearance, theme, accent });
            const style = getComputedStyle(document.documentElement);
            const value = (name: string) => style.getPropertyValue(name).trim();
            const ratio = (foreground: string, background: string) => { const a = luminance(value(foreground)), b = luminance(value(background)); return Number(((Math.max(a, b) + .05) / (Math.min(a, b) + .05)).toFixed(2)); };
            return { accent, theme, buttonContrast: ratio('--button-text', '--button-bg'), accentTextContrast: ratio('--accent', '--surface'), terminalContrast: Object.fromEntries(['black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white', 'bright-black', 'bright-red', 'bright-green', 'bright-yellow', 'bright-blue', 'bright-magenta', 'bright-cyan', 'bright-white'].map(color => [color, ratio(`--terminal-${color}`, '--surface')])) };
          });
          appearance.applyAppearance({ ...appearance.defaultAppearance, theme, motion: 'reduced' });
          return results;
        }, theme);
        await page.evaluate(() => { document.documentElement.style.zoom = '2'; });
        await capture('css-zoom-200');
        scenes.zoomMethod = 'CSS zoom 2 on isolated test document; not a claim of OS/browser zoom control.';
      }
      writeFileSync(`${directory}/${viewport.name}-${theme}-observations.json`, JSON.stringify({ errors, scenes }, null, 2));
      expect(errors).toEqual([]);
    });
  }
}
