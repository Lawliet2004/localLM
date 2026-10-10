import { expect, test } from '@playwright/test';

test('search settings explain an outage and allow checking again after recovery', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/src/main.tsx*', route => route.fulfill({
    contentType: 'application/javascript', body: "import '/e2e/web-search-preview.tsx';",
  }));
  await page.route('**/src/lib/api.ts*', route => route.fulfill({
    contentType: 'application/javascript',
    body: `
      export const nativeAvailable = true;
      export const errorMessage = String;
      let checks = 0;
      export const api = {
        getWebSearchConfig: async () => ({ provider: 'searxng', searxngBaseUrl: 'http://127.0.0.1:8080', hasGoogleApiKey: false, googleCxId: '', searchFallbackEnabled: true }),
        getExecutionConfig: async () => ({ pythonPath: '', nodePath: '', powershellPath: '' }),
        hasDaytonaKey: async () => false,
        pendingDaytonaOperations: async () => [],
                readiness: async () => ({step:'ready',message:'',nextAction:null}),
        qualificationResults: async () => null,
        profiles: async () => [],
        tasksList: async () => [],
        webSearchHealth: async () => ({ search: checks++ ? 'healthy' : 'unreachable', google: 'not_configured' }),
      };
    `,
  }));
  await page.goto('/');
  const check = page.getByRole('button', { name: 'Check connection', exact: true });
  await check.click();
  await expect(page.getByRole('status')).toContainText('Check that the service is running and the saved URL is correct.');
  await expect(page.getByRole('status')).toContainText('Google fallback also needs an API key');
  await expect(check).toBeEnabled();
  await check.click();
  await expect(page.getByRole('status')).toHaveText('Search: healthy');
  expect(errors).toEqual([]);
});
