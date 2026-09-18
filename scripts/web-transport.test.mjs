import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isPublicAddress, PinnedPageFetcher } from './web-transport.mjs';
test('retrieval rejects every non-public address family', async () => {
  for (const ip of ['127.0.0.1', '10.2.3.4', '169.254.169.254', '100.64.0.1', '192.0.2.1', '::1', '::ffff:7f00:1', 'fe80::1', 'fd00::2', '2001:db8::1']) {
    assert.equal(isPublicAddress(ip), false, ip);
  }
  assert.equal(isPublicAddress('1.1.1.1'), true);
  assert.equal(isPublicAddress('2606:4700:4700::1111'), true);
  const result = await new PinnedPageFetcher().fetch('http://127.0.0.1/secret');
  assert.equal(result.success, false);
});
test('public redirect cannot reach a private destination', async () => {
  const calls = [];
  const fetcher = new PinnedPageFetcher(2, 1, async url => {
    calls.push(url.href);
    return url.pathname === '/robots.txt' ? {status:404,headers:{},body:''}
      : {status:302,headers:{location:'http://127.0.0.1/secrets'},body:''};
  });
  const result = await fetcher.fetch('https://1.1.1.1/article');
  assert.equal(result.success, false);
  assert.match(result.error, /Non-public/);
  assert.equal(calls.some(url => url.includes('127.0.0.1')), false);
});
test('robots disallow prevents the article request', async () => {
  const calls = [];
  const fetcher = new PinnedPageFetcher(2, 1, async url => {
    calls.push(url.pathname);
    return {status:200,headers:{'content-type':'text/plain'},body:'User-agent: *\nDisallow: /private'};
  });
  const result = await fetcher.fetch('https://1.1.1.1/private/article');
  assert.equal(result.success, false);
  assert.deepEqual(calls, ['/robots.txt']);
});
