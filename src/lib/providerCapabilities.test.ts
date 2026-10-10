import { expect, it } from 'vitest';
import { localInferenceProvider } from './providerCapabilities';
it('allows loopback providers and rejects remote hosts and deceptive URLs', () => {
  for (const baseUrl of ['http://127.0.0.1:8080/v1', 'http://127.5.6.7', 'http://LOCALHOST.:8080', 'http://[::1]:8080']) expect(localInferenceProvider({ baseUrl })).toBe(true);
  for (const baseUrl of ['https://api.openai.com/v1', 'http://localhost@evil.com', 'http://user:pass@127.0.0.1', 'http://127.0.0.1.evil.com', 'file://localhost/model', 'http://[::ffff:127.0.0.1]', 'invalid']) expect(localInferenceProvider({ baseUrl })).toBe(false);
});
