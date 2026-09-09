import { chromium, expect } from '@playwright/test';
import { writeFileSync } from 'node:fs';

const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
let page;
await expect.poll(() => { page = browser.contexts()[0].pages().find(value => value.url().includes('1420')); return Boolean(page); }).toBe(true);
page.setDefaultTimeout(15000);
const invoke = (command, args = {}) => page.evaluate(async ({ command, args }) => {
  const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
  return invoke(command, args);
}, { command, args });
const report = { testedAt: new Date().toISOString(), skills: [] };
try {
  const installedBefore = (await invoke('list_skills')).filter(item => item.installed).map(item => item.id);
  for (const id of ['gh-fix-ci', 'jupyter-notebook']) {
    if (!installedBefore.includes(id)) await invoke('install_skill', { id });
  }
  for (const id of ['gh-fix-ci', 'jupyter-notebook']) {
    const rows = await invoke('skill_dependencies', { id });
    report.skills.push({ id, rows });
  }
  await page.getByRole('button', { name: 'Skills', exact: true }).click();
  const card = page.locator('details.catalog-item').filter({ has: page.locator('strong', { hasText: 'Gh Fix Ci' }) });
  await card.locator(':scope > summary').scrollIntoViewIfNeeded();
  await card.locator(':scope > summary').click();
  const check = card.getByRole('button', { name: 'Check dependencies', exact: true });
  await check.scrollIntoViewIfNeeded();
  await check.click();
  await expect(card.getByText('Dependencies', { exact: true })).toBeVisible();
  await expect(card.locator('.skill-dependencies').getByText('github', { exact: true })).toBeVisible();
  await page.screenshot({ path: 'test-results/skill-dependencies.png' });
  report.screenshot = 'test-results/skill-dependencies.png';

  const gh = report.skills.find(item => item.id === 'gh-fix-ci').rows;
  const nb = report.skills.find(item => item.id === 'jupyter-notebook').rows;
  expect(gh.map(row => `${row.dependency.kind}:${row.dependency.name}`).sort()).toEqual(['connector:github', 'externalCli:gh', 'interpreter:python']);
  expect(nb.map(row => `${row.dependency.kind}:${row.dependency.name}`).sort()).toEqual(['externalCli:uv', 'interpreter:python']);
  for (const row of [...gh, ...nb]) {
    expect(row.remedy.length).toBeGreaterThan(10);
    expect(row.dependency.detail.length).toBeGreaterThan(5);
  }
  expect(gh.find(row => row.dependency.name === 'github').satisfied).toBe(false);
  expect(gh.find(row => row.dependency.name === 'github').remedy).toContain('Connect github in Connectors');
  writeFileSync('test-results/skill-dependencies-smoke.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report.skills.map(item => ({ id: item.id, rows: item.rows.map(row => ({ dep: `${row.dependency.kind}:${row.dependency.name}`, satisfied: row.satisfied })) }))));
} finally {
  await browser.close();
}
