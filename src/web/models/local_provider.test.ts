import { afterEach, describe, expect, it, vi } from 'vitest';
import { LocalOpenAICompatibleProvider } from './local_provider';

const originalFetch = globalThis.fetch;
afterEach(() => {
  vi.unstubAllGlobals();
  globalThis.fetch = originalFetch;
});

function mockServer(propsBody: unknown) {
  const bodies: Record<string, unknown>[] = [];
  vi.stubGlobal('fetch', async (input: any, init?: RequestInit) => {
    const url = String(input.url ?? input);
    if (url.endsWith('/props')) return new Response(JSON.stringify(propsBody), { status: 200 });
    if (url.endsWith('/chat/completions')) {
      bodies.push(JSON.parse(String(init?.body)));
      return new Response(JSON.stringify({
        choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }],
        usage: { total_tokens: 5 },
      }), { status: 200 });
    }
    return new Response('{}', { status: 404 });
  });
  return bodies;
}

describe('LocalOpenAICompatibleProvider reasoning controls', () => {
  it('sends enable_thinking=false on low-effort calls when the template supports it', async () => {
    const bodies = mockServer({ chat_template: "{% if enable_thinking is defined and enable_thinking %}{% endif %}" });
    const provider = new LocalOpenAICompatibleProvider({ baseUrl: 'http://127.0.0.1:9000/v1', maxInputTokens: 100000 });
    await provider.generate({ systemPrompt: 's', userPrompt: 'u', reasoningEffort: 'low' });
    expect(bodies[0].chat_template_kwargs).toEqual({ enable_thinking: false });
  });

  it('prefers reasoning_effort=low when the template exposes it', async () => {
    const bodies = mockServer({ chat_template: '{{ reasoning_effort }} {{ enable_thinking }}' });
    const provider = new LocalOpenAICompatibleProvider({ baseUrl: 'http://127.0.0.1:9000/v1', maxInputTokens: 100000 });
    await provider.generate({ systemPrompt: 's', userPrompt: 'u', reasoningEffort: 'low' });
    expect(bodies[0].chat_template_kwargs).toEqual({ reasoning_effort: 'low' });
  });

  it('sends no kwargs when the template lacks controls or the probe fails', async () => {
    const bodies = mockServer({ chat_template: 'plain' });
    const provider = new LocalOpenAICompatibleProvider({ baseUrl: 'http://127.0.0.1:9000/v1', maxInputTokens: 100000 });
    await provider.generate({ systemPrompt: 's', userPrompt: 'u', reasoningEffort: 'low' });
    expect(bodies[0].chat_template_kwargs).toBeUndefined();

    vi.stubGlobal('fetch', async () => { throw new Error('down'); });
    const offline = new LocalOpenAICompatibleProvider({ baseUrl: 'http://127.0.0.1:9001/v1', maxInputTokens: 100000 });
    await expect(offline.generate({ systemPrompt: 's', userPrompt: 'u', reasoningEffort: 'low' })).rejects.toThrow();
  });

  it('leaves default-effort calls untouched', async () => {
    const bodies = mockServer({ chat_template: '{{ enable_thinking }}' });
    const provider = new LocalOpenAICompatibleProvider({ baseUrl: 'http://127.0.0.1:9000/v1', maxInputTokens: 100000 });
    await provider.generate({ systemPrompt: 's', userPrompt: 'u' });
    expect(bodies[0].chat_template_kwargs).toBeUndefined();
  });
});
