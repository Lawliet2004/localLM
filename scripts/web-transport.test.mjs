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

test('HTTP error reads retain their status and a bounded diagnostic body', async () => {
  const get = async (url) => {
    if (url.pathname === '/robots.txt') return { status: 404, headers: {}, body: Buffer.from('') };
    const status = url.pathname === '/missing' ? 404 : 403;
    return {
      status,
      headers: { 'content-type': 'text/html' },
      body: Buffer.from(`<html><title>${status} Error</title><p>${'diagnostic '.repeat(20)}</p></html>`),
    };
  };
  const fetcher = new PinnedPageFetcher(2, 1, get);
  for (const [path, status] of [['/missing', 404], ['/blocked', 403]]) {
    const result = await fetcher.fetch(`https://1.1.1.1${path}`, { maxBytes: 4096 });
    assert.equal(result.success, false);
    assert.equal(result.status, status);
    assert.match(result.error, new RegExp(String(status)));
    assert.match(result.body, /diagnostic/);
    assert.ok(Buffer.byteLength(result.body) <= 4096);
  }
});

test('HTTP 200 reads remain successful and retain their content', async () => {
  const get = async (url) => url.pathname === '/robots.txt'
    ? { status: 404, headers: {}, body: Buffer.from('') }
    : { status: 200, headers: { 'content-type': 'text/html' }, body: Buffer.from('<html><title>Article</title><p>Fetched article text.</p></html>') };
  const result = await new PinnedPageFetcher(2, 1, get).fetch('https://1.1.1.1/article');
  assert.equal(result.success, true);
  assert.equal(result.status, 200);
  assert.match(result.body, /Fetched article text/);
});

test('a cold host starts immediately and later requests retain host spacing', async () => {
  const starts = [];
  const fetcher = new PinnedPageFetcher(2, 1, async () => {
    starts.push(Date.now());
    return { status: 200, headers: {}, body: Buffer.from('page') };
  });
  const before = Date.now();
  await Promise.all([fetcher.limited(new URL('https://1.1.1.1/first'), {}), fetcher.limited(new URL('https://1.1.1.1/second'), {})]);
  assert.ok(starts[0] - before < 500, 'cold host should not wait a full second');
  assert.ok(starts[1] - starts[0] >= 990, 'same-host starts remain spaced');
  assert.equal(fetcher.queues.size, 0, 'completed tasks are released');
});

test('Retry-After retains HTTP diagnostics without issuing requests inside the cooldown', async () => {
  let calls = 0;
  const fetcher = new PinnedPageFetcher(2, 1, async () => {
    calls++;
    return { status: 429, headers: { 'retry-after': '60' }, body: Buffer.from('rate limited') };
  });
  const url = new URL('https://1.1.1.1/article');
  assert.equal((await fetcher.limited(url, {})).status, 429);
  await assert.rejects(fetcher.limited(url, {}), /Retry-After cooldown/);
  assert.equal(calls, 1);
  assert.ok(fetcher.cooldowns.get(url.hostname) > Date.now() + 59000);
});

test('failed robots requests can recover and maxRetries zero makes only one attempt', async () => {
  let calls = 0;
  const fetcher = new PinnedPageFetcher(2, 1, async () => {
    if (++calls === 1) throw new Error('temporary transport failure');
    return { status: 404, headers: {}, body: Buffer.from('') };
  });
  const url = new URL('https://1.1.1.1/article');
  await assert.rejects(fetcher.allowed(url, { maxRetries: 0 }), /temporary transport failure/);
  assert.equal(calls, 1);
  assert.equal(await fetcher.allowed(url, { maxRetries: 0 }), true);
  assert.equal(calls, 2);
});

test('unavailable robots policies are checked again after the host recovers', async () => {
  let calls = 0;
  const fetcher = new PinnedPageFetcher(2, 1, async () => ++calls === 1
    ? { status: 503, headers: {}, body: Buffer.from('unavailable') }
    : { status: 200, headers: {}, body: Buffer.from('User-agent: *\nDisallow: /private') });
  const url = new URL('https://1.1.1.1/private/article');
  assert.equal(await fetcher.allowed(url, {}), true);
  fetcher.cooldowns.set(url.hostname, Date.now() - 1);
  fetcher.nextRequestAt.set(url.hostname, Date.now() - 1);
  assert.equal(await fetcher.allowed(url, {}), false);
  assert.equal(calls, 2, 'transient HTTP failures must not become permanent allow-all policies');
});
