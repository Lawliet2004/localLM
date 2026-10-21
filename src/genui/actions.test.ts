import { describe, expect, it } from 'vitest';
import { presentationAction } from './actions';
import { buttonAction } from './catalog';

describe('presentation action allowlist', () => {
  it.each(['javascript:alert(1)', 'data:text/html,test', 'file:///etc/passwd', 'tauri://localhost', 'http://example.com', 'https://user:secret@example.com', 'not a URL'])('rejects unsafe URLs: %s', url => {
    expect(presentationAction({ type: 'open_url', params: { url }, humanFriendlyMessage: 'Open' })).toBeNull();
    expect(buttonAction.safeParse({ type: 'open_url', url }).success).toBe(false);
  });
  it('accepts an https URL without credentials', () => {
    expect(presentationAction({ type: 'open_url', params: { url: 'https://example.com/docs' }, humanFriendlyMessage: 'Open' }))
      .toEqual({ type: 'open_url', url: 'https://example.com/docs' });
  });
  it.each(['run_shell', 'invoke', 'execute', 'custom', 'open_path', '__proto__'])('rejects unlisted actions: %s', type => {
    expect(presentationAction({ type, params: { command: 'whoami' }, humanFriendlyMessage: 'Run' })).toBeNull();
  });
  it('bounds follow-ups and preserves visible message text', () => {
    expect(presentationAction({ type: 'continue_conversation', params: { message: 'Explain TCP' }, humanFriendlyMessage: 'More' }))
      .toEqual({ type: 'continue_conversation', message: 'Explain TCP' });
    for (const message of ['', ' ', 'x'.repeat(4001), {}, null]) {
      expect(presentationAction({ type: 'continue_conversation', params: { message }, humanFriendlyMessage: '' })).toBeNull();
    }
  });
});
