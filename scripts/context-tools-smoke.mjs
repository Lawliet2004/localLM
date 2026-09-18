// Native acceptance. Start LocalLM with CDP enabled and Bonsai loaded at 8192.
// LOCALLM_CDP_URL can override the default WebView debugging endpoint.
import { chromium, expect } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';

const browser = await chromium.connectOverCDP(process.env.LOCALLM_CDP_URL ?? 'http://127.0.0.1:9223');
try {
  const page = browser.contexts().flatMap(context => context.pages())
    .find(page => /^http:\/\/(localhost|127\.0\.0\.1):1420\//.test(page.url()));
  if (!page) throw new Error('LocalLM native webview is not open.');
  const title = `Context and tools verification ${Date.now()}`;
  await page.evaluate(async title => {
    const { api } = await import('/src/lib/api.ts');
    const state = await api.bootstrap();
    if (state.runtime.phase !== 'ready' || state.runtime.loadedConfig.contextLength !== 8192) {
      throw new Error('Load Bonsai with 8192 context before this test.');
    }
    const connectors = await api.listConnectors();
    if (connectors.some(item => ['parallel-web', 'deepwiki'].includes(item.id) && item.connected)) {
      throw new Error('This test requires Parallel Web and DeepWiki to be disconnected.');
    }
    const conversation = await api.createConversation();
    await api.saveConversationModel(conversation.id, { providerId: null, modelId: '' });
    await api.saveConversationTools(conversation.id, {
      accessMode: 'ask', sources: [], tools: [
        { connectorId: 'parallel-web', toolName: 'web_search' },
        { connectorId: 'parallel-web', toolName: 'web_fetch' },
        { connectorId: 'deepwiki', toolName: 'ask_question' },
        { connectorId: 'deepwiki', toolName: 'read_wiki_contents' },
        { connectorId: 'deepwiki', toolName: 'read_wiki_structure' },
      ],
    });
    await api.renameConversation(conversation.id, title);
  }, title);
  await page.reload();
  await page.getByRole('button', { name: title, exact: true }).click();
  const prompt = `Reference padding (ignore): ${'blue '.repeat(2200)}\nWhat is 17 + 25? Answer with just the number. /no_think`;
  await page.getByRole('textbox', { name: 'Message', exact: true }).fill(prompt);
  await page.getByRole('button', { name: 'Send message', exact: true }).click();
  await expect(page.getByText(/Unavailable for this reply: parallel-web, deepwiki/)).toBeVisible();
  await expect(page.locator('.message-assistant .markdown')).toContainText('42', { timeout: 120000 });
  await expect(page.getByRole('alert')).toHaveCount(0);
  const usage = await page.getByLabel('Last request context').innerText();
  const counts = usage.match(/([\d,]+) input \+ ([\d,]+) response reserve \/ ([\d,]+) context/);
  expect(counts).not.toBeNull();
  const [input, reserve, capacity] = counts.slice(1).map(value => Number(value.replaceAll(',', '')));
  expect(input + reserve).toBeGreaterThan(4096);
  expect(input + reserve).toBeLessThanOrEqual(capacity);
  expect(capacity).toBe(8192);
  mkdirSync('.local', { recursive: true });
  await page.screenshot({ path: '.local/context-tools-regression.png' });
  const report = { input, reserve, capacity, answer: '42', disconnectedTools: 'skipped with visible notice' };
  writeFileSync('.local/context-tools-regression.json', JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report));
} finally {
  await browser.close();
}
