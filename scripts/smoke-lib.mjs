// Shared preamble for the *-smoke scripts: attach to the debug WebView2
// instance over CDP and return the app page once it appears.
import { chromium, expect } from '@playwright/test';

export async function connectSmoke(cdpUrl = 'http://127.0.0.1:9223') {
  const browser = await chromium.connectOverCDP(cdpUrl);
  let page;
  await expect.poll(() => {
    page = browser.contexts()[0].pages().find(p => p.url().includes('1420') || p.url().includes('tauri.localhost'));
    return Boolean(page);
  }).toBe(true);
  return { browser, page };
}

// Navigate the app to a Settings section through the rail (opens Settings from
// chat first when needed).
export async function openSettingsSection(page, section) {
  const item = page.getByRole('button', { name: section, exact: true });
  if (!(await item.isVisible().catch(() => false))) {
    await page.getByRole('button', { name: 'Settings', exact: true }).click();
  }
  await item.click();
}

// Invoke a Tauri command inside the webview.
export function invokeTauri(page, command, args = {}) {
  return page.evaluate(
    async ({ command, args }) => {
      const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
      return invoke(command, args);
    },
    { command, args }
  );
}
