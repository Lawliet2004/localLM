import { chromium } from '@playwright/test';

const browser = await chromium.connectOverCDP('http://127.0.0.1:9223');
const page = browser.contexts()[0].pages().find(p => p.url().includes('1420'));
if (!page) {
  console.log('No 1420 page found');
  await browser.close();
  process.exit(0);
}

const data = await page.evaluate(async () => {
  const { invoke } = await import('/node_modules/@tauri-apps/api/core.js');
  const b = await invoke('bootstrap');
  const convId = 'fa01ad24-5bdc-4101-b143-13c29e44f569';
  const messages = await invoke('get_messages', { id: convId });
  return messages;
});

for (let i = 0; i < data.length; i++) {
  const m = data[i];
  console.log(`=== Message ${i}: ${m.role} (${m.status}) ===`);
  console.log('Error:', m.error);
  if (m.role === 'tool') {
    try {
      const parsed = JSON.parse(m.content);
      console.log('Request:', JSON.stringify(parsed.request, null, 2));
      console.log('Result type:', typeof parsed.result);
      if (parsed.result && typeof parsed.result === 'object') {
        console.log('Result isError:', parsed.result.isError);
        console.log('Result keys:', Object.keys(parsed.result));
        if (parsed.result.message) console.log('Result message:', parsed.result.message);
      } else {
        console.log('Result preview:', String(parsed.result).slice(0, 300));
      }
    } catch (e) {
      console.log('Content slice:', m.content.slice(0, 300));
    }
  } else {
    console.log('Content:', m.content);
  }
}

await browser.close();
