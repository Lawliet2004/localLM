import { expect, it } from 'vitest';
import { detectChallenge } from './challenge';

it('detects Cloudflare interstitial markers in an HTTP 200 body', () => {
  const html = '<html><head><title>Just a moment...</title></head><body>cf-chl challenge</body></html>';
  expect(detectChallenge(200, html)?.marker).toBe('title:just-a-moment');
});

it('does not treat ordinary English "just a moment" as a challenge', () => {
  const html = '<html><title>Install guide</title><p>Wait just a moment while the service starts.</p></html>';
  expect(detectChallenge(200, html)).toBeNull();
});

it('detects challenge-platform tokens', () => {
  expect(detectChallenge(200, '<div id="challenge-platform"></div>')?.marker).toBe('challenge-platform');
});
