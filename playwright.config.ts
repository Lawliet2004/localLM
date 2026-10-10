import { defineConfig } from '@playwright/test';
// @ts-expect-error type error without @types/node package
import process from 'node:process';

export default defineConfig({
  testDir: './e2e', fullyParallel: false,
  workers: 2,
  expect: { timeout: 10_000 },
  use: { baseURL: 'http://127.0.0.1:1420', channel: 'msedge', viewport: { width: 1280, height: 840 }, screenshot: 'only-on-failure' },
  // Vite only — the Tauri app must not be rebuilt or restarted by tests.
  webServer: { command: 'npx vite --host 127.0.0.1 --port 1420 --strictPort', url: 'http://127.0.0.1:1420', reuseExistingServer: !process.env.CI },
});
