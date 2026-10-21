import { expect, test, type Page } from '@playwright/test';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';

test.use({ screenshot: 'off', reducedMotion: 'reduce' });

async function setup(page: Page, theme = 'dark', stopped = false) {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.addInitScript(theme => localStorage.setItem('locallm-appearance', JSON.stringify({ theme, accent: 'amber', motion: 'reduced' })), theme);
  const fixture = readFileSync('e2e/fixtures/audit-api.js', 'utf8') + '\n' + readFileSync('e2e/fixtures/review-api.js', 'utf8') + `
api.localReasoningCapability=async()=>"enable_thinking";
const originalBootstrap=api.bootstrap;
api.bootstrap=async()=>{const value=await originalBootstrap();return {...value,${stopped ? 'runtime:{phase:"stopped",message:"No model loaded",modelPath:null},' : ''}rememberedTools:{sources:[],tools:[],accessMode:"fullAccess"}}};`;
  await page.route('**/src/lib/api.ts*', route => route.fulfill({ contentType: 'application/javascript', body: fixture }));
  await page.route(/.*@tauri-apps_api_window\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const getCurrentWindow=()=>({isMaximized:async()=>false,onResized:async()=>()=>{}})' }));
  await page.route(/.*@tauri-apps_plugin-dialog\.js.*/, route => route.fulfill({ contentType: 'application/javascript', body: 'export const open=async()=>null;export const save=async()=>null;export const confirm=async()=>true;' }));
  await page.goto('/');
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toBeEnabled();
  await page.evaluate(() => document.fonts.ready);
  return errors;
}

test('response options preserve the draft and choices, dismiss on Escape and restore focus', async ({ page }) => {
  const errors = await setup(page);
  const draft = page.getByRole('textbox', { name: 'Message', exact: true });
  await draft.fill('A cleaner place to write.');
  const trigger = page.getByRole('button', { name: 'Response options', exact: true });
  await trigger.focus();
  await page.keyboard.press('Enter');
  const panel = page.getByRole('dialog', { name: 'Response options', exact: true });
  await expect(panel.getByRole('button', { name: 'Close response options' })).toBeFocused();
  await page.getByLabel('Reasoning for next response', { exact: true }).selectOption('quick');
  await panel.getByRole('button', { name: 'Plan mode', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Plan mode', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Escape');
  await expect(panel).not.toBeVisible();
  await expect(trigger).toBeFocused();
  await expect(draft).toHaveValue('A cleaner place to write.');
  await trigger.click();
  await expect(page.getByLabel('Reasoning for next response', { exact: true })).toHaveValue('quick');
  await draft.click({ position: { x: (await draft.boundingBox())!.width - 12, y: 12 } });
  await expect(panel).not.toBeVisible();
  expect(errors).toEqual([]);
});

test('closing response options cancels an active microphone and rejects late results', async ({ page }) => {
  await page.addInitScript(() => {
    const fixture = { aborts: 0, session: null as any };
    (window as any).__composerSpeech = fixture;
    class Recognition {
      continuous = false; interimResults = false; lang = '';
      onresult: ((event: any) => void) | null = null; onerror = null; onend: (() => void) | null = null;
      start() { fixture.session = this; }
      stop() { this.onend?.(); }
      abort() { fixture.aborts++; this.onend?.(); }
    }
    Object.defineProperty(window, 'SpeechRecognition', { configurable: true, value: Recognition });
  });
  await setup(page);
  await page.getByRole('button', { name: 'Response options', exact: true }).click();
  await page.getByLabel('Voice input', { exact: true }).click();
  await page.getByRole('button', { name: 'Start microphone', exact: true }).click();
  await page.keyboard.press('Escape');
  await expect.poll(() => page.evaluate(() => (window as any).__composerSpeech.aborts)).toBe(1);
  await page.evaluate(() => (window as any).__composerSpeech.session.onresult?.({ resultIndex: 0, results: [{ isFinal: true, 0: { transcript: 'Late transcript' } }] }));
  await expect(page.getByRole('textbox', { name: 'Message', exact: true })).toHaveValue('');
});

test('response options and primary controls stay inside a narrow window', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 640 });
  const errors = await setup(page, 'light', true);
  await page.getByRole('button', { name: 'Response options', exact: true }).click();
  const panel = page.getByRole('dialog', { name: 'Response options', exact: true });
  await page.getByLabel('Voice input', { exact: true }).click();
  const bounds = await panel.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(16);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(374);
  expect(bounds!.y).toBeGreaterThanOrEqual(8);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(632);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: 'Add files', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Permission mode: Full access', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Response options', exact: true }).click();
  await page.setViewportSize({ width: 600, height: 640 });
  await expect(panel).not.toBeVisible();
  expect(errors).toEqual([]);
});

if (process.env.COMPOSER_VISUAL_ROUND) {
  const directory = `.impeccable/review/composer-cleanup-${process.env.COMPOSER_VISUAL_ROUND}`;
  for (const width of [1325, 390]) for (const theme of ['light', 'dark']) {
    test(`composer visual ${width} ${theme}`, async ({ page }) => {
      mkdirSync(directory, { recursive: true });
      await page.setViewportSize({ width, height: width > 500 ? 900 : 844 });
      const errors = await setup(page, theme, true);
      await page.getByRole('textbox', { name: 'Message', exact: true }).fill('I make');
      await page.screenshot({ path: `${directory}/${width}-${theme}-closed.png`, animations: 'disabled' });
      await page.getByRole('button', { name: 'Response options', exact: true }).click();
      await page.screenshot({ path: `${directory}/${width}-${theme}-options.png`, animations: 'disabled' });
      const observations = await page.evaluate(() => ({ documentOverflow: document.documentElement.scrollWidth > innerWidth, composerOverflow: document.querySelector('.composer')!.scrollWidth > document.querySelector('.composer')!.clientWidth, runningAnimations: document.getAnimations().filter(animation => animation.playState === 'running').length,
        textFont: getComputedStyle(document.querySelector('textarea')!).fontFamily, chatFont: getComputedStyle(document.documentElement).getPropertyValue('--chat-font'), permissionColor: getComputedStyle(document.querySelector('.permission-trigger')!).color }));
      writeFileSync(`${directory}/${width}-${theme}.json`, JSON.stringify({ errors, ...observations }, null, 2));
      expect(errors).toEqual([]);
    });
  }
}
