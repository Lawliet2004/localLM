import { afterEach, expect, it, vi } from 'vitest';
import { LocalOpenAICompatibleProvider } from './local_provider';

afterEach(() => vi.unstubAllGlobals());

it('uses exact chat input tokens and serializes calls for one inference slot', async () => {
  let active = 0; let maximum = 0;
  const fetchMock = vi.fn(async (input: string | URL) => {
    if (String(input).endsWith('/input_tokens')) return Response.json({ input_tokens: 10 });
    if (String(input).endsWith('/tokenize')) return Response.json({ tokens: Array(200).fill(1) });
    active++; maximum = Math.max(active, maximum);
    await new Promise(resolve => setTimeout(resolve, 20)); active--;
    return Response.json({ choices: [{ message: { content: 'ok' } }] });
  });
  vi.stubGlobal('fetch', fetchMock);
  const provider = new LocalOpenAICompatibleProvider({ maxInputTokens: 100 });
  const results = await Promise.all([1, 2, 3].map(() => provider.generate({ systemPrompt: 'extract', userPrompt: 'facts' })));
  expect(results.map(result => result.text)).toEqual(['ok', 'ok', 'ok']); expect(maximum).toBe(1);
  expect(fetchMock.mock.calls.some(call => String(call[0]).endsWith('/input_tokens'))).toBe(true);
});

it('rejects credential-bearing and non-loopback inference URLs', () => {
  for (const baseUrl of ['http://user:pass@localhost/v1', 'http://localhost@evil.com', 'http://127.0.0.1.evil.com', 'http://[::ffff:127.0.0.1]', 'file://localhost/v1']) {
    expect(() => new LocalOpenAICompatibleProvider({ baseUrl })).toThrow('loopback');
  }
});

it('sends the configured worker slot and leaves single-slot requests unpinned', async () => {
  const payloads: Array<Record<string, unknown>> = [];
  vi.stubGlobal('fetch', async (input: string | URL, init?: RequestInit) => {
    if (String(input).endsWith('/input_tokens')) return Response.json({ input_tokens: 1 });
    if (String(input).endsWith('/tokenize')) return Response.json({ tokens: [1] });
    payloads.push(JSON.parse(String(init?.body)));
    return Response.json({ choices: [{ message: { content: 'ok' } }] });
  });
  const request = { systemPrompt: 'extract', userPrompt: 'facts' };
  const options = { baseUrl: 'http://localhost:1919/v1', idSlot: 1 };
  await new LocalOpenAICompatibleProvider(options).generate(request);
  await new LocalOpenAICompatibleProvider().generate(request);
  expect(payloads[0].id_slot).toBe(1); expect(payloads[1]).not.toHaveProperty('id_slot');
});

it('disables thinking only for short research calls that request it', async () => {
  const payloads: Array<Record<string, unknown>> = [];
  vi.stubGlobal('fetch', async (input: string | URL, init?: RequestInit) => {
    if (String(input).endsWith('/input_tokens')) return Response.json({ input_tokens: 1 });
    if (String(input).endsWith('/tokenize')) return new Response('', { status: 404 });
    payloads.push(JSON.parse(String(init?.body)) as Record<string, unknown>);
    return Response.json({ choices: [{ message: { content: 'S1.1' }, finish_reason: 'stop' }] });
  });
  const provider = new LocalOpenAICompatibleProvider({ baseUrl: 'http://127.0.0.1:1234/v1' });
  const request = { systemPrompt: 'Extract facts.', userPrompt: 'Evidence: S1.1', maxTokens: 220 };
  await provider.generate({ ...request, disableThinking: true });
  await provider.generate(request);
  expect(payloads[0].chat_template_kwargs).toEqual({ enable_thinking: false });
  expect(payloads[1]).not.toHaveProperty('chat_template_kwargs');
});
