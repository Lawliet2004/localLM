import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  renderPage,
  findEdgeBinary,
  createMediationSession,
  gateUrl,
  lastLauncherError,
  ISOLATED_CONTEXT_OPTIONS,
} from './web-render.mjs';

test('findEdgeBinary picks the first existing candidate', () => {
  const found = findEdgeBinary(['missing.exe', 'present.exe'], (p) => p === 'present.exe');
  assert.equal(found, 'present.exe');
  assert.equal(findEdgeBinary(['nope'], () => false), null);
});

test('renderPage refuses non-public and non-HTTP targets before spawning', async () => {
  // The browser resolves DNS itself, so the SSRF check must run first —
  // a missing binary would prove the check was skipped by spawning anyway.
  assert.equal(await renderPage('http://127.0.0.1:8080/internal', { edgePath: 'nonexistent.exe' }), null);
  assert.equal(await renderPage('file:///C:/Windows/win.ini', { edgePath: 'nonexistent.exe' }), null);
  assert.equal(await renderPage('not a url', { edgePath: 'nonexistent.exe' }), null);
});

test('renderPage returns null when the browser binary is missing', async () => {
  assert.equal(
    await renderPage('https://example.org/', { edgePath: 'C:\\definitely-not-edge\\msedge.exe', timeoutMs: 5000 }),
    null,
  );
});

const HTML = `<!doctype html><html><head><title>rendered</title></head><body>${'visible text '.repeat(30)}</body></html>`;

function publicLookup(address = '1.1.1.1') {
  const calls = [];
  return {
    calls,
    lookup: async (host) => {
      calls.push(host);
      return [{ address, family: address.includes(':') ? 6 : 4 }];
    },
  };
}

function sessionWith(get, lookup, limits = {}) {
  return createMediationSession({
    get,
    lookup,
    timeoutMs: 5000,
    maxBytes: 1024 * 1024,
    ...limits,
  });
}

test('raw Edge dump is not a fallback and routing is not treated as a sandbox', () => {
  const source = readFileSync(new URL('./web-render.mjs', import.meta.url), 'utf8');
  assert.equal(source.includes('--dump-dom'), false);
  assert.equal(source.includes('child_process'), false);
  assert.equal(source.includes('.continue('), false);
  assert.equal(source.includes('.fetch('), false);
  assert.match(source, /not an OS sandbox/);
  assert.equal(ISOLATED_CONTEXT_OPTIONS.serviceWorkers, 'block');
  assert.equal(ISOLATED_CONTEXT_OPTIONS.acceptDownloads, false);
});

test('alternate IP forms and mapped addresses never reach the transport', async () => {
  const fetched = [];
  const looked = [];
  const get = async (url) => { fetched.push(url.href); throw new Error('fetched'); };
  const lookup = async (host) => { looked.push(host); throw new Error('looked up'); };
  const urls = [
    'http://2130706433/',
    'http://0x7f000001/',
    'http://0177.0.0.1/',
    'http://0x7f.0x0.0x0.0x1/',
    'http://017700000001/',
    'http://0xA9FEA9FE/',
    'http://010.1.1.1/',
    'http://0x01010101/',
    'http://127.1/',
    'http://[::ffff:127.0.0.1]/',
    'http://[::ffff:7f00:1]/',
    'http://[::ffff:1.1.1.1]/',
    'http://[0:0:0:0:0:ffff:127.0.0.1]/',
  ];
  for (const url of urls) {
    assert.equal(await renderPage(url, { get, lookup, edgePath: 'missing.exe', exists: () => true }), null, url);
    const reason = gateUrl(url);
    assert.ok(reason, url);
    assert.match(reason, /alternate IP form blocked|IPv4-mapped IPv6 blocked|Non-public address blocked/, url);
  }
  assert.deepEqual(fetched, []);
  assert.deepEqual(looked, []);
});

test('canonical public IPv4 is fetched and a mapped form of the same address is not', async () => {
  const fetched = [];
  const session = sessionWith(async (url) => {
    fetched.push(url.href);
    return { status: 200, headers: { 'content-type': 'text/html' }, body: HTML };
  }, async () => { throw new Error('literal addresses are not looked up'); });
  const allowed = await session.exchange('http://1.1.1.1/ok');
  assert.equal(allowed.ok, true);
  assert.equal(allowed.finalUrl, 'http://1.1.1.1/ok');
  assert.equal(allowed.origin, 'http://1.1.1.1');
  const mapped = await session.exchange('http://[::ffff:1.1.1.1]/ok');
  assert.equal(mapped.ok, false);
  assert.match(mapped.error, /IPv4-mapped/);
  assert.deepEqual(fetched, ['http://1.1.1.1/ok']);
});

test('public redirect cannot reach a private or obfuscated destination', async () => {
  const cases = [
    'http://127.0.0.1/secret',
    'http://10.1.2.3/secret',
    'http://169.254.169.254/latest',
    'http://2130706433/secret',
    'http://0x7f000001/secret',
    'http://0177.0.0.1/secret',
    'http://[::ffff:127.0.0.1]/secret',
    'http://[::ffff:1.1.1.1]/still-mapped',
    'http://010.1.1.1/',
    'http://0x01010101/',
    'http://metadata.google.internal/',
    'file:///etc/passwd',
  ];
  for (const location of cases) {
    const fetched = [];
    const session = sessionWith(async (url) => {
      fetched.push(url.href);
      return { status: 302, headers: { location }, body: '' };
    }, async () => [{ address: '1.1.1.1', family: 4 }]);
    const result = await session.exchange('https://public.example/start');
    assert.equal(result.ok, false, location);
    assert.equal(fetched.length, 1, location);
    assert.equal(fetched[0], 'https://public.example/start');
    assert.equal(fetched.some((href) => /127\.0\.0\.1|10\.1\.2\.3|169\.254|2130706433|0x7f|0177|ffff|010\.1\.1\.1|0x01010101|metadata|passwd/i.test(href)), false, location);
  }
});

test('DNS is re-resolved on every redirect and a later private answer is not fetched', async () => {
  const fetched = [];
  const looked = [];
  let n = 0;
  const session = sessionWith(async (url) => {
    fetched.push(url.href);
    return { status: 302, headers: { location: '/next' }, body: '' };
  }, async (host) => {
    looked.push(host);
    n += 1;
    return [{ address: n === 1 ? '1.1.1.1' : '127.0.0.1', family: 4 }];
  });
  const result = await session.exchange('https://rebind.example/start');
  assert.equal(result.ok, false);
  assert.match(result.error, /Non-public/);
  assert.deepEqual(fetched, ['https://rebind.example/start']);
  assert.deepEqual(looked, ['rebind.example', 'rebind.example']);
});

test('a name that resolves to both a public and a private address is blocked', async () => {
  const fetched = [];
  const session = sessionWith(async (url) => {
    fetched.push(url.href);
    return { status: 200, headers: { 'content-type': 'text/html' }, body: HTML };
  }, async () => [{ address: '1.1.1.1', family: 4 }, { address: '10.0.0.1', family: 4 }]);
  const result = await session.exchange('https://dual.example/');
  assert.equal(result.ok, false);
  assert.match(result.error, /Non-public/);
  assert.deepEqual(fetched, []);
});

test('redirects keep the real final URL and origin', async () => {
  const looked = [];
  const session = sessionWith(async (url) => {
    if (url.hostname === 'start.example') {
      return { status: 302, headers: { location: 'https://landed.example/final' }, body: '' };
    }
    return { status: 200, headers: { 'content-type': 'text/html' }, body: HTML };
  }, async (host) => {
    looked.push(host);
    return [{ address: '1.1.1.1', family: 4 }];
  });
  const result = await session.exchange('https://start.example/entry');
  assert.equal(result.ok, true);
  assert.equal(result.finalUrl, 'https://landed.example/final');
  assert.equal(result.origin, 'https://landed.example');
  assert.deepEqual(looked, ['start.example', 'landed.example']);
});

test('request, byte, time, and concurrency limits fail closed', async () => {
  const requestFetches = [];
  const requestSession = sessionWith(async (url) => {
    requestFetches.push(url.pathname);
    return { status: 302, headers: { location: '/next' }, body: '' };
  }, async () => [{ address: '1.1.1.1', family: 4 }], { maxRequests: 1, maxRedirects: 5 });
  const requestLimited = await requestSession.exchange('https://limit.example/start');
  assert.equal(requestLimited.ok, false);
  assert.match(requestLimited.error, /request limit/);
  assert.deepEqual(requestFetches, ['/start']);

  const byteSession = sessionWith(async () => ({
    status: 200, headers: { 'content-type': 'text/html' }, body: 'x'.repeat(80),
  }), async () => [{ address: '1.1.1.1', family: 4 }], { maxBytes: 40 });
  const byteLimited = await byteSession.exchange('https://limit.example/big');
  assert.equal(byteLimited.ok, false);
  assert.match(byteLimited.error, /byte limit/);

  let clock = 0;
  const timeFetches = [];
  const timeSession = createMediationSession({
    timeoutMs: 1000,
    maxBytes: 10000,
    now: () => clock,
    lookup: async () => [{ address: '1.1.1.1', family: 4 }],
    get: async (url) => {
      timeFetches.push(url.pathname);
      clock = 5000;
      return { status: 302, headers: { location: '/later' }, body: '' };
    },
  });
  const timed = await timeSession.exchange('https://limit.example/a');
  assert.equal(timed.ok, false);
  assert.match(timed.error, /time limit/);
  assert.deepEqual(timeFetches, ['/a']);

  let release;
  const held = new Promise((resolve) => { release = resolve; });
  let started;
  const startedGate = new Promise((resolve) => { started = resolve; });
  const concurrentFetches = [];
  const concurrent = createMediationSession({
    timeoutMs: 5000,
    maxBytes: 10000,
    maxConcurrency: 1,
    lookup: async () => [{ address: '1.1.1.1', family: 4 }],
    get: async (url) => {
      concurrentFetches.push(url.pathname);
      started();
      await held;
      return { status: 200, headers: { 'content-type': 'text/html' }, body: HTML };
    },
  });
  const first = concurrent.exchange('https://a.example/one');
  await startedGate;
  const second = await concurrent.exchange('https://b.example/two');
  assert.equal(second.ok, false);
  assert.match(second.error, /concurrency limit/);
  assert.deepEqual(concurrentFetches, ['/one']);
  release();
  assert.equal((await first).ok, true);
});

test('private iframes, websockets, downloads, popups, and service workers are not fetched', async () => {
  const fetched = [];
  const session = sessionWith(async (url) => {
    fetched.push(String(url.href || url));
    return {
      status: 200,
      headers: { 'content-type': 'text/html', 'content-disposition': 'attachment; filename=x.html' },
      body: HTML,
    };
  }, async () => [{ address: '1.1.1.1', family: 4 }]);

  const iframe = await session.mediate({ url: 'http://192.168.1.9/frame.html', resourceType: 'document' });
  assert.equal(iframe.action, 'abort');
  assert.match(iframe.reason, /Non-public/);

  for (const request of [
    { url: 'wss://public.example/socket', resourceType: 'websocket', isWebSocket: true },
    { url: 'https://public.example/file', resourceType: 'download', isDownload: true },
    { url: 'https://public.example/popup', resourceType: 'document', isPopup: true },
    { url: 'https://public.example/sw.js', resourceType: 'serviceworker', isServiceWorker: true },
  ]) {
    const decision = await session.mediate(request);
    assert.equal(decision.action, 'abort', request.resourceType);
    assert.equal(decision.action === 'fulfill', false);
  }
  assert.deepEqual(fetched, []);

  const download = await session.mediate({ url: 'https://public.example/report', resourceType: 'document' });
  assert.equal(download.action, 'abort');
  assert.match(download.reason, /download blocked/);
  assert.deepEqual(fetched, ['https://public.example/report']);
});

test('route handler fulfills only mediated responses and aborts a private subresource', async () => {
  const fetched = [];
  const session = sessionWith(async (url) => {
    fetched.push(url.href);
    return { status: 200, headers: { 'content-type': 'text/javascript' }, body: 'export {}' };
  }, async () => [{ address: '1.1.1.1', family: 4 }]);
  const calls = { continue: 0, fetch: 0, fulfill: 0, abort: 0 };
  const routeFor = (url, resourceType) => ({
    request: () => ({ url: () => url, resourceType: () => resourceType, headers: () => ({}) }),
    continue: async () => { calls.continue += 1; },
    fetch: async () => { calls.fetch += 1; throw new Error('route.fetch is unchecked'); },
    fulfill: async () => { calls.fulfill += 1; },
    abort: async () => { calls.abort += 1; },
  });
  const allowed = await session.handleRoute(routeFor('https://cdn.example/app.js', 'script'));
  assert.equal(allowed.action, 'fulfill');
  assert.equal(allowed.finalUrl, 'https://cdn.example/app.js');
  assert.equal(allowed.origin, 'https://cdn.example');
  const blocked = await session.handleRoute(routeFor('http://10.0.0.8/iframe.html', 'document'));
  assert.equal(blocked.action, 'abort');
  assert.match(blocked.reason, /Non-public/);
  assert.equal(calls.continue, 0);
  assert.equal(calls.fetch, 0);
  assert.equal(calls.fulfill, 1);
  assert.equal(calls.abort, 1);
  assert.deepEqual(fetched, ['https://cdn.example/app.js']);
});

test('launch failure is recorded and the fetched document is not returned', async () => {
  let fetched = 0;
  const result = await renderPage('https://example.org/article', {
    edgePath: 'C:\\definitely-not-edge\\msedge.exe',
    exists: () => true,
    timeoutMs: 5000,
    lookup: async () => [{ address: '1.1.1.1', family: 4 }],
    get: async () => {
      fetched += 1;
      return { status: 200, headers: { 'content-type': 'text/html' }, body: HTML };
    },
    launchBrowser: async () => { throw new Error('edge launch failed'); },
  });
  assert.equal(result, null);
  assert.match(lastLauncherError(), /edge launch failed/);
  assert.equal(fetched, 1);
});

test('mediated render uses an isolated context, the post-redirect origin, and no direct navigation', async () => {
  const order = [];
  const fetched = [];
  const looked = [];
  let contextOptions;
  let persistent = false;
  let navigated = 0;
  let socket;
  const page = {
    url: () => 'about:blank',
    goto: async () => { navigated += 1; throw new Error('direct navigation'); },
    setContent: async (html) => { order.push('setContent'); assert.match(html, /<base href="https:\/\/landed\.example\/final">/); },
    waitForLoadState: async () => {},
    content: async () => HTML,
    on: (event) => { order.push(`on:${event}`); },
    close: async () => {},
  };
  const context = {
    newPage: async () => page,
    route: async (pattern) => { order.push(`route:${pattern}`); },
    routeWebSocket: async (pattern, handler) => { order.push(`ws:${pattern}`); socket = handler; },
    on: (event) => { order.push(`context:${event}`); },
    close: async () => {},
  };
  const browser = {
    newContext: async (options) => { contextOptions = options; return context; },
    launchPersistentContext: async () => { persistent = true; throw new Error('persistent context is forbidden'); },
    close: async () => {},
  };
  let launchSpec;
  const result = await renderPage('https://start.example/entry', {
    edgePath: 'msedge.exe',
    exists: () => true,
    timeoutMs: 5000,
    virtualTimeBudgetMs: 20,
    lookup: async (host) => {
      looked.push(host);
      return [{ address: '1.1.1.1', family: 4 }];
    },
    get: async (url) => {
      fetched.push(url.href);
      if (url.hostname === 'start.example') return { status: 302, headers: { location: 'https://landed.example/final' }, body: '' };
      return { status: 200, headers: { 'content-type': 'text/html' }, body: HTML };
    },
    launchBrowser: async (spec) => { launchSpec = spec; return browser; },
  });
  assert.equal(result.finalUrl, 'https://landed.example/final');
  assert.equal(result.origin, 'https://landed.example');
  assert.notEqual(result.finalUrl, 'about:blank');
  assert.equal(result.html, HTML);
  assert.equal(navigated, 0);
  assert.equal(persistent, false);
  assert.equal(contextOptions.serviceWorkers, 'block');
  assert.equal(contextOptions.acceptDownloads, false);
  assert.equal(contextOptions.proxy.server, 'http://127.0.0.1:1');
  assert.equal(launchSpec.executablePath, 'msedge.exe');
  assert.equal(launchSpec.url, undefined);
  assert.equal(JSON.stringify(launchSpec).includes('start.example'), false);
  assert.equal(JSON.stringify(launchSpec).includes('landed.example'), false);
  assert.ok(order.indexOf('route:**/*') < order.indexOf('setContent'));
  assert.ok(order.indexOf('ws:**/*') < order.indexOf('setContent'));
  assert.deepEqual(looked, ['start.example', 'landed.example']);
  assert.deepEqual(fetched, ['https://start.example/entry', 'https://landed.example/final']);
  const closed = { close: 0, connectToServer: 0 };
  socket({ close: () => { closed.close += 1; }, connectToServer: () => { closed.connectToServer += 1; } });
  assert.equal(closed.close, 1);
  assert.equal(closed.connectToServer, 0);
  assert.equal(lastLauncherError(), null);
});

test('a private second hop does not launch the browser', async () => {
  let launched = false;
  const fetched = [];
  let n = 0;
  const result = await renderPage('https://rebind.example/start', {
    edgePath: 'msedge.exe',
    exists: () => true,
    timeoutMs: 5000,
    lookup: async () => {
      n += 1;
      return [{ address: n === 1 ? '1.1.1.1' : '10.2.3.4', family: 4 }];
    },
    get: async (url) => {
      fetched.push(url.href);
      return { status: 302, headers: { location: 'https://rebind.example/private' }, body: '' };
    },
    launchBrowser: async () => { launched = true; throw new Error('should not launch'); },
  });
  assert.equal(result, null);
  assert.equal(launched, false);
  assert.deepEqual(fetched, ['https://rebind.example/start']);
  assert.equal(lastLauncherError(), null);
});
