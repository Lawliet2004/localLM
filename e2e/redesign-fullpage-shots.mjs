// Full-page Appearance screenshots — run against the Vite dev server on :1420.
//   node e2e/redesign-fullpage-shots.mjs
import { chromium } from '@playwright/test';

const BASE = 'http://127.0.0.1:1420';
const OUT = 'test-results/redesign';

async function shot(name, { appearance, width = 1280, height = 800 } = {}) {
  const browser = await chromium.launch({ channel: 'msedge' });
  try {
    const page = await browser.newPage({ viewport: { width, height } });
    if (appearance) {
      await page.addInitScript(saved => localStorage.setItem('locallm-appearance', JSON.stringify(saved)), appearance);
    }
    await page.goto(BASE, { waitUntil: 'commit' });
    await page.getByRole('button', { name: 'Send message' }).waitFor();
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
    await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Appearance', exact: true }).click();
    await page.getByRole('heading', { name: 'Appearance', level: 1 }).waitFor();
    // .settings-content is an inner scroller, so fullPage alone stays
    // viewport-sized: grow the viewport to the page's real content height.
    const contentHeight = await page.evaluate(() => document.querySelector('.settings-content')?.scrollHeight ?? 0);
    if (contentHeight > height) await page.setViewportSize({ width, height: contentHeight });
    await page.waitForTimeout(350);
    await page.screenshot({ path: `${OUT}/${name}.png` });
    console.log(`${name}.png`);
  } finally {
    await browser.close();
  }
}

await shot('appearance-full-dark');
await shot('appearance-full-light', { appearance: { theme: 'light' } });
await shot('appearance-full-custom', {
  appearance: {
    accent: 'ruby', darkStock: 'ink', density: 'compact', userMessage: 'plain',
    chatFont: 'serif', contrast: 'high', radius: 'sharp',
  },
});
