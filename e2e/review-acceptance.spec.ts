import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';

// Functional acceptance only. The implementation task owns the batched visual
// review, so these tests deliberately do not create screenshot-polish rounds.
test.use({ screenshot: 'off' });

async function setup(page: Page, populated = false) {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.route('**/src/lib/api.ts*', route => route.fulfill({ contentType: 'application/javascript',
    body: readFileSync('e2e/fixtures/audit-api.js', 'utf8') + '\n' + readFileSync('e2e/fixtures/review-api.js', 'utf8') }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const getCurrentWindow=()=>({isMaximized:async()=>false,onResized:async()=>()=>{}})' }));
  await page.route(/.*@tauri-apps_plugin-dialog\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const open=async()=>null;export const save=async options=>{window.__review.saveDialogOptions=options;return window.__reviewCancelSave?null:"/fixture/saved/result-16.html";};export const confirm=async()=>true;' }));
  await page.route('https://assets.example.test/chart.svg', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100" fill="#356349"/></svg>' }));
  await page.goto('/');
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeEnabled();
  if (populated) {
    if ((page.viewportSize()?.width ?? 1280) < 760) await page.getByRole('button', { name: 'Expand sidebar', exact: true }).click();
    await page.getByRole('button', { name: 'Review acceptance conversation', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Answer ready', exact: true })).toBeVisible();
    if ((page.viewportSize()?.width ?? 1280) < 760) await page.getByRole('button', { name: 'Collapse sidebar', exact: true }).click();
  }
  return errors;
}

test('completed answer is readable before collapsed reasoning and activity, with visible deduplicated sources', async ({ page }) => {
  const errors = await setup(page, true);
  const reasoning = page.getByTestId('turn-reasoning');
  const activity = page.locator('.activity-summary');
  await expect(reasoning).not.toHaveAttribute('open');
  await expect(activity).not.toHaveAttribute('open');
  await expect(reasoning.locator('summary')).toHaveText('Reasoning');
  await expect(activity.locator(':scope > summary')).toHaveText('2 tools used');
  const sources = page.getByRole('region', { name: 'Answer sources' });
  await expect(sources.getByRole('link')).toHaveCount(1);
  await expect(sources.getByRole('link')).toHaveAttribute('href', /^https:\/\/evidence\.example\.test\/report/);
  await expect(sources).toContainText('[A] [B]');
  await expect(sources).toContainText('2026-10-17');
  await expect(sources.getByText('Invalid source')).toHaveCount(0);
  await expect(page.locator('.message-assistant table')).toContainText('Alpha');
  await expect(page.getByRole('button', { name: 'Copy code block' })).toBeVisible();
  await expect(page.getByText(/Thought for 8s|Explored 1 item/)).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('image preview opens from keyboard, contains focus, and restores it on Escape', async ({ page }) => {
  const errors = await setup(page, true);
  const trigger = page.getByRole('button', { name: 'Enlarge Fixture chart', exact: true });
  await trigger.focus();
  await page.keyboard.press('Enter');
  const modal = page.getByRole('dialog', { name: 'Fixture chart', exact: true });
  await expect(modal).toBeVisible();
  await expect(modal.getByRole('button', { name: 'Close preview', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  // Native dialogs may move focus to browser chrome; background page controls
  // must stay inert. Browser chrome is not a dialog focus escape into the app.
  await expect.poll(() => modal.evaluate(element => !document.hasFocus() || element.contains(document.activeElement))).toBe(true);
  await page.keyboard.press('Escape');
  await expect(modal).not.toBeVisible();
  await expect(trigger).toBeFocused();
  expect(errors).toEqual([]);
});

test('command palette has keyboard selection, Escape restoration and working actions', async ({ page }) => {
  const errors = await setup(page);
  const composer = page.getByRole('textbox', { name: 'Message', exact: true });
  await composer.focus();
  await page.keyboard.press('Control+Shift+p');
  const palette = page.getByRole('dialog', { name: 'Command palette', exact: true });
  const query = palette.getByRole('combobox', { name: 'Search commands' });
  await expect(query).toBeFocused();
  const first = await query.getAttribute('aria-activedescendant');
  await page.keyboard.press('ArrowDown');
  await expect(query).not.toHaveAttribute('aria-activedescendant', first!);
  await expect(palette.getByRole('option', { selected: true })).toHaveCount(1);
  await page.keyboard.press('Escape');
  await expect(palette).not.toBeVisible();
  await expect(composer).toBeFocused();
  await page.keyboard.press('Control+Shift+p');
  await query.fill('appearance');
  await page.keyboard.press('Enter');
  await expect(palette).not.toBeVisible();
  await expect(page.getByRole('heading', { name: 'Appearance', exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test('palette keyboard wrap keeps the active option visible in a short window', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 480 });
  await setup(page);
  await page.keyboard.press('Control+Shift+p');
  const palette = page.getByRole('dialog', { name: 'Command palette' });
  await page.keyboard.press('ArrowUp');
  await expect(palette.getByRole('option', { selected: true })).toBeInViewport();
});

test('conversation search failure retains query and retries without pretending there were no matches', async ({ page }) => {
  const errors = await setup(page);
  await page.keyboard.press('Control+k');
  const query = page.getByRole('searchbox', { name: 'Search conversations', exact: true });
  await query.fill('needle');
  await expect(page.getByRole('alert')).toContainText('Conversation search failed');
  await expect(query).toHaveValue('needle');
  await expect(page.getByText('No matching conversations found.', { exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Retry search', exact: true }).click();
  await expect(page.getByRole('button', { name: /Recovered result/ })).toBeVisible();
  await expect(page.getByRole('alert')).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__review.searchAttempts)).toBe(2);
  expect(errors).toEqual([]);
});

test('generated output overflow remains reachable and HTML source never executes in the app', async ({ page }) => {
  const errors = await setup(page, true);
  await page.locator('.activity-summary > summary').click();
  const generated = page.getByRole('region', { name: 'Generated files' });
  await expect(generated.getByRole('button', { name: 'Preview exports/result-16.html', exact: true })).toHaveCount(0);
  await generated.getByRole('button', { name: 'Show all 17 files', exact: true }).click();
  await generated.getByRole('button', { name: 'Preview exports/result-16.html', exact: true }).click();
  const artifact = generated.getByRole('region', { name: 'Artifact exports/result-16.html', exact: true });
  await expect(artifact).toBeVisible();
  await expect(artifact).toContainText('<script>parent.__review.escapedArtifact=true</script>');
  await expect(artifact.getByRole('button', { name: 'Source', exact: true })).toHaveAttribute('aria-pressed', 'false');
  await artifact.getByRole('button', { name: 'Source', exact: true }).click();
  await expect(artifact.getByRole('button', { name: 'Source', exact: true })).toHaveAttribute('aria-pressed', 'true');
  expect(await page.evaluate(() => (window as any).__review.escapedArtifact)).toBe(false);
  await artifact.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(artifact.getByRole('status')).toContainText('Saved /fixture/saved/result-16.html');
  expect(await page.evaluate(() => (window as any).__review.saveDialogOptions.defaultPath)).toBe('result-16.html');
  expect(await page.evaluate(() => (window as any).__review.savedCopies)).toEqual([{ path: '/fixture/saved/result-16.html', content: '<h1>Literal artifact</h1><script>parent.__review.escapedArtifact=true</script>', mimeType: 'text/html' }]);
  expect(await page.evaluate(() => (window as any).__review.previewCalls)).toEqual([{ root: '/fixture/a', path: 'exports/result-16.html', sha256: 'a'.repeat(64) }]);
  await artifact.getByRole('button', { name: 'Close exports/result-16.html preview', exact: true }).click();
  await expect(artifact).toHaveCount(0);
  expect(errors).toEqual([]);
});

test('cancelling artifact Save never calls the native file writer', async ({ page }) => {
  await page.addInitScript(() => { (window as any).__reviewCancelSave = true; });
  await setup(page, true);
  await page.locator('.activity-summary > summary').click();
  await page.getByRole('button', { name: 'Preview exports/result-00.csv', exact: true }).click();
  const artifact = page.getByRole('region', { name: 'Artifact exports/result-00.csv', exact: true });
  await artifact.getByRole('button', { name: 'Save', exact: true }).click();
  expect(await page.evaluate(() => (window as any).__review.savedCopies)).toEqual([]);
  await expect(artifact).toBeVisible();
});

test('CSV artifact switches between real table preview and complete source', async ({ page }) => {
  await setup(page, true);
  await page.locator('.activity-summary > summary').click();
  await page.getByRole('button', { name: 'Preview exports/result-00.csv', exact: true }).click();
  const artifact = page.getByRole('region', { name: 'Artifact exports/result-00.csv', exact: true });
  await expect(artifact.getByRole('table')).toContainText('Alpha');
  await artifact.getByRole('button', { name: 'Source', exact: true }).click();
  await expect(artifact.getByRole('table')).toHaveCount(0);
  await expect(artifact.locator('pre')).toHaveText('Name,Latency\nAlpha,12\nBeta,18\n');
});

test('artifact Open displays readable content in an expanded desktop view', async ({ page }) => {
  await setup(page, true);
  await page.locator('.activity-summary > summary').click();
  await page.getByRole('button', { name: 'Preview exports/result-00.csv', exact: true }).click();
  const artifact = page.getByRole('region', { name: 'Artifact exports/result-00.csv', exact: true });
  const trigger = artifact.getByRole('button', { name: 'Open', exact: true });
  await trigger.click();
  const expanded = page.getByRole('dialog', { name: 'Expanded exports/result-00.csv', exact: true });
  await expect(expanded).toBeVisible();
  await expect(expanded).toContainText('Name,Latency');
  await expect(expanded).toContainText('Alpha,12');
  await expect(expanded.getByRole('button', { name: 'Close expanded preview' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(expanded).not.toBeVisible();
  await expect(trigger).toBeFocused();
});

async function fakeSpeech(page: Page, supported = true) {
  await page.addInitScript(enabled => {
    const fixture = { starts: 0, aborts: 0, session: null as any };
    (window as any).__voice = fixture;
    class Recognition {
      continuous = false; interimResults = false; lang = '';
      onresult: ((event: unknown) => void) | null = null;
      onerror: ((event: unknown) => void) | null = null;
      onend: (() => void) | null = null;
      start() { fixture.starts++; fixture.session = this; }
      stop() { this.onend?.(); }
      abort() { fixture.aborts++; this.onend?.(); }
    }
    Object.defineProperty(window, 'SpeechRecognition', { configurable: true, value: enabled ? Recognition : undefined });
    Object.defineProperty(window, 'webkitSpeechRecognition', { configurable: true, value: undefined });
  }, supported);
}

test('voice requires an explicit start and adds a transcript to the draft without sending it', async ({ page }) => {
  await fakeSpeech(page);
  const errors = await setup(page);
  expect(await page.evaluate(() => (window as any).__voice.starts)).toBe(0);
  await page.getByRole('button', { name: 'Response options', exact: true }).click();
  await page.getByLabel('Voice input', { exact: true }).click();
  await expect(page.getByText(/may send audio to its provider/)).toBeVisible();
  expect(await page.evaluate(() => (window as any).__voice.starts)).toBe(0);
  await page.getByRole('button', { name: 'Start microphone', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Listening…' })).toBeVisible();
  await page.evaluate(() => {
    const session = (window as any).__voice.session;
    session.onresult?.({ resultIndex: 0, results: [{ isFinal: true, 0: { transcript: 'Review this dictated draft.' } }] });
    session.onend?.();
  });
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('Review this dictated draft.');
  expect(await page.evaluate(() => (window as any).__audit.sends)).toBe(0);
  expect(errors).toEqual([]);
});

test('voice cancellation aborts recognition and rejects later results', async ({ page }) => {
  await fakeSpeech(page);
  await setup(page);
  await page.getByRole('button', { name: 'Response options', exact: true }).click();
  await page.getByLabel('Voice input', { exact: true }).click();
  await page.getByRole('button', { name: 'Start microphone', exact: true }).click();
  await page.getByRole('button', { name: 'Cancel recording', exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__voice.aborts)).toBe(1);
  await page.evaluate(() => (window as any).__voice.session.onresult?.({ resultIndex: 0, results: [{ isFinal: true, 0: { transcript: 'Cancelled input' } }] }));
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('');
  await expect(page.getByRole('button', { name: 'Start microphone', exact: true })).toBeVisible();
});

test('changing conversations cancels dictation so its transcript cannot enter another draft', async ({ page }) => {
  await fakeSpeech(page);
  await setup(page);
  await page.getByRole('button', { name: 'Response options', exact: true }).click();
  await page.getByLabel('Voice input', { exact: true }).click();
  await page.getByRole('button', { name: 'Start microphone', exact: true }).click();
  await page.getByRole('button', { name: 'Review acceptance conversation', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Answer ready', exact: true })).toBeVisible();
  await expect.poll(() => page.evaluate(() => (window as any).__voice.aborts)).toBe(1);
  await page.evaluate(() => (window as any).__voice.session.onresult?.({ resultIndex: 0, results: [{ isFinal: true, 0: { transcript: 'Transcript from the previous conversation' } }] }));
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('');
});

test('unsupported speech webviews explain the limitation without offering a broken microphone action', async ({ page }) => {
  await fakeSpeech(page, false);
  await setup(page);
  await page.getByRole('button', { name: 'Response options', exact: true }).click();
  await page.getByLabel('Voice input', { exact: true }).click();
  await expect(page.getByText(/Voice transcription is unavailable in this browser or desktop webview/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Start microphone', exact: true })).toHaveCount(0);
});

test('tool result validation failure remains prominent even when the process exits zero', async ({ page }) => {
  await page.addInitScript(() => { (window as any).__reviewFailedExecution = true; });
  const errors = await setup(page, true);
  await expect(page.getByTestId('status-failed')).toBeVisible();
  await expect(page.getByTestId('status-failed')).toHaveClass(/failure/);
  await expect(page.getByTestId('status-failed')).toContainText('Exit 0');
  await expect(page.locator('.command-run-card.failed .status-badge.success')).toHaveCount(0);
  await expect(page.locator('.command-run-card.failed')).toBeVisible();
  expect(errors).toEqual([]);
});

test('Latest never overlaps interactive composer controls in a narrow window', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await setup(page, true);
  await page.locator('.chat-scroll').evaluate(element => { element.scrollTop = 0; });
  const latest = page.getByRole('button', { name: 'Jump to latest message', exact: true });
  await expect(latest).toBeVisible();
  const collisions = await latest.evaluate(button => {
    const bounds = button.getBoundingClientRect();
    return [...document.querySelectorAll<HTMLElement>('.composer-region button,.composer-region input,.composer-region select,.composer-region textarea,.composer-region summary')].flatMap(element => {
      if (element === button) return [];
      const other = element.getBoundingClientRect();
      const overlap = Math.min(bounds.right, other.right) - Math.max(bounds.left, other.left) > 1 && Math.min(bounds.bottom, other.bottom) - Math.max(bounds.top, other.top) > 1;
      return other.width && other.height && overlap ? [element.getAttribute('aria-label') || element.textContent?.trim() || element.tagName] : [];
    });
  });
  expect(collisions).toEqual([]);
});

test('expanded web results reject unsafe links and keep source titles readable in narrow windows', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await setup(page, true);
  await page.locator('.activity-summary > summary').click();
  const web = page.getByTestId('command-run-card').filter({ hasText: 'Searched the web' });
  await web.locator(':scope > summary').click();
  await expect(web.locator('a[href^="javascript:"]')).toHaveCount(0);
  await expect(web.getByRole('link', { name: 'Invalid source', exact: true })).toHaveCount(0);
  const source = web.getByRole('link', { name: 'Fixture evidence report', exact: true });
  await expect(source).toHaveCount(1);
  await expect(source).toBeVisible();
  expect(await source.evaluate(element => element.scrollWidth <= element.clientWidth + 1)).toBe(true);
});
