// Redesign screenshot capture — run against the Vite dev server on :1420.
//   node e2e/redesign-screenshots.mjs
import { chromium } from '@playwright/test';

const BASE = 'http://127.0.0.1:1420';
const OUT = 'test-results/redesign';

async function shot(name, { appearance, width = 1280, height = 800, setup } = {}) {
  // A fresh browser per shot: this Edge channel stops navigating after a
  // few pages in one process (goto/reload hang regardless of waitUntil).
  const browser = await chromium.launch({ channel: 'msedge' });
  try {
    const page = await browser.newPage({ viewport: { width, height } });
    if (appearance) {
      await page.addInitScript(saved => localStorage.setItem('locallm-appearance', JSON.stringify(saved)), appearance);
    }
    await page.goto(BASE, { waitUntil: 'commit' });
    await page.getByRole('button', { name: 'Send message' }).waitFor();
    if (setup) await setup(page);
    await page.waitForTimeout(350); // let fonts + the settle animation finish
    await page.screenshot({ path: `${OUT}/${name}.png` });
    console.log(`${name}.png`);
  } finally {
    await browser.close();
  }
}

async function openAppearance(page) {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Appearance', exact: true }).click();
  await page.getByRole('heading', { name: 'Appearance', level: 1 }).waitFor();
}

// 1. Main window — dark, umber + amber, empty welcome
await shot('main-dark-umber-welcome');

// 2. Main window — light, fiber
await shot('main-light-fiber-welcome', { appearance: { theme: 'light' } });

// 3. Appearance page — dark
await shot('appearance-dark', { setup: openAppearance });

// 4. Appearance page — light (fiber)
await shot('appearance-light', { appearance: { theme: 'light' }, setup: openAppearance });

// 5. Appearance — ruby + ink + compact + plain messages
await shot('appearance-ruby-ink-compact-plain', {
  appearance: { accent: 'ruby', darkStock: 'ink', density: 'compact', userMessage: 'plain' },
  setup: openAppearance,
});

// 6. Appearance at 600px
await shot('appearance-600px', { width: 600, setup: openAppearance });

// 7. Conversation transcript (existing fixture)
await shot('conversation-dark', { setup: async page => {
  await page.evaluate(async () => {
    const { mountChatPreview } = await import('/e2e/chat-preview.tsx');
    mountChatPreview();
  });
  await page.getByText('Worked for 3m 51s').waitFor();
} });

// 8. Permission popover — dark, Full access selected.
//    nativeAvailable is false in a plain browser, so mount the control
//    directly with the prop forced on (same trick as chat-preview.tsx).
await shot('permission-popover-full-access', { setup: async page => {
  await page.evaluate(async () => {
    const { mountPermissionPreview } = await import('/e2e/permission-preview.tsx');
    mountPermissionPreview();
  });
  await page.getByRole('button', { name: /Permission mode: Full access/ }).click();
  await page.getByRole('dialog', { name: 'Select permission mode' }).waitFor();
} });
