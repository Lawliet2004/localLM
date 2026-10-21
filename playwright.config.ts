import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e', fullyParallel: false,
  use: { baseURL: 'http://127.0.0.1:1420', channel: process.env.PLAYWRIGHT_CHANNEL || (process.platform === 'win32' ? 'msedge' : 'chrome'), viewport: { width: 1280, height: 840 }, screenshot: 'only-on-failure' },
  webServer: { command: 'npm run dev -- --host 127.0.0.1', url: 'http://127.0.0.1:1420', reuseExistingServer: !process.env.CI },
});
