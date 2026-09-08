import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = browser.contexts()[0].pages().find(value => value.url().includes('1420'));
if (!page) throw new Error('Native LocalLM webview is unavailable.');
page.setDefaultTimeout(15000);
try {
  await page.getByRole('button', { name: 'Skills', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Search skills' }).fill('wiki-qa');
  const item = page.locator('.catalog-item');
  if (await item.getAttribute('open') === null) await item.locator('summary').first().click();
  if (await item.getByRole('button', { name: 'Install', exact: true }).count()) await item.getByRole('button', { name: 'Install', exact: true }).click();
  await expect(item.getByRole('button', { name: 'Read instructions' })).toBeVisible({ timeout: 120000 });
  await item.getByRole('button', { name: 'Read instructions' }).click();
  await expect(item.locator('.skill-preview pre')).toContainText('wiki-qa');
  if (await item.getByRole('button', { name: 'Activate', exact: true }).count()) await item.getByRole('button', { name: 'Activate', exact: true }).click();
  await expect(item.getByRole('button', { name: 'Deactivate', exact: true })).toBeVisible();
  await page.reload();
  await page.getByRole('button', { name: 'Skills', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Search skills' }).fill('wiki-qa');
  await item.locator('summary').first().click();
  await expect(item.getByRole('button', { name: 'Deactivate', exact: true })).toBeVisible();
  await item.getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(item.getByRole('button', { name: 'Install', exact: true })).toBeVisible();
  const ids = await page.evaluate(async () => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return (await invoke('list_skills')).map(item => item.id); });
  for (const id of ids) {
    await page.evaluate(async id => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); await invoke('install_skill', { id }); const text = await invoke('read_skill_file', { id, path: 'SKILL.md' }); if (!text.startsWith('---')) throw new Error(`Missing skill frontmatter: ${id}`); }, id);
    console.log(`Verified native install: ${id}`);
  }
  const skills = await page.evaluate(async () => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); return await invoke('list_skills'); });
  expect(skills).toHaveLength(13);
  expect(skills.every(item => item.installed && !item.active)).toBe(true);
  await page.reload();
  await page.getByRole('button', { name: 'Skills', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Search skills' }).fill('wiki-qa');
  await item.locator('summary').first().click();
  await item.getByRole('button', { name: 'Read instructions' }).click();
  mkdirSync('test-results', { recursive: true });
  await page.screenshot({ path: 'test-results/native-skills.png' });
  await item.getByRole('button', { name: 'Activate', exact: true }).click();
  await expect(item.getByRole('button', { name: 'Deactivate', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Models & runtime', exact: true }).click();
  if (await page.getByRole('button', { name: 'Load model', exact: true }).count()) {
    await page.getByRole('button', { name: 'Load model', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Unload', exact: true })).toBeVisible({ timeout: 120000 });
  }
  await page.getByRole('button', { name: 'New conversation', exact: false }).click();
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill('Summarize only the Response Format section of my active wiki-qa skill. Name the required table and diagram syntax. Do not inspect any repository.');
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Stop response' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Stop response' })).toHaveCount(0, { timeout: 180000 });
  const answer = await page.locator('.message-assistant .markdown').innerText();
  expect(answer).toContain('Key Files'); expect(answer.toLowerCase()).toContain('mermaid');
  await expect(page.getByRole('alert')).toHaveCount(0);
  await page.evaluate(async () => { const { invoke } = await import('/node_modules/@tauri-apps/api/core.js'); await invoke('set_skill_active', { id: 'wiki-qa', active: false }); });
  const report = { testedAt: new Date().toISOString(), lifecycle: 'install / inspect / activate / webview reload / remove / reinstall verified', guidanceAnswer: answer, skills: skills.map(({ id, revision, files }) => ({ id, revision, fileCount: files.length })) };
  writeFileSync('test-results/skills-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally { await browser.close(); }
