import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BrowserPanel, closeBrowserForTab, resolveAddress } from './BrowserPanel';

const mocks = vi.hoisted(() => ({
  browserShow: vi.fn(() => Promise.resolve()),
  browserHide: vi.fn(() => Promise.resolve()),
  browserClose: vi.fn(() => Promise.resolve()),
  browserNavigate: vi.fn(() => Promise.resolve()),
  browserUrl: vi.fn(() => Promise.resolve({ url: 'about:blank' })),
  browserReload: vi.fn(),
  browserGoBack: vi.fn(),
  browserGoForward: vi.fn(),
  openUrl: vi.fn(),
}));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
  api: {
    browserShow: mocks.browserShow,
    browserHide: mocks.browserHide,
    browserClose: mocks.browserClose,
    browserNavigate: mocks.browserNavigate,
    browserUrl: mocks.browserUrl,
    browserReload: mocks.browserReload,
    browserGoBack: mocks.browserGoBack,
    browserGoForward: mocks.browserGoForward,
  },
}));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: mocks.openUrl }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));

// jsdom lacks ResizeObserver, and checkVisibility (called from the panel's
// MutationObserver callback) is a no-op stub.
globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} } as unknown as typeof ResizeObserver;
if (!HTMLElement.prototype.checkVisibility) HTMLElement.prototype.checkVisibility = () => false;

describe('resolveAddress', () => {
  it('returns a full http(s) URL unchanged', () => {
    expect(resolveAddress('https://example.com/page?q=1')).toEqual({ url: 'https://example.com/page?q=1' });
    expect(resolveAddress('HTTP://EXAMPLE.COM')).toEqual({ url: 'HTTP://EXAMPLE.COM' });
  });

  it('prefixes a bare domain with https', () => {
    expect(resolveAddress('example.com')).toEqual({ url: 'https://example.com' });
    expect(resolveAddress('sub.example.co.uk/path')).toEqual({ url: 'https://sub.example.co.uk/path' });
    expect(resolveAddress('example.com:8080/p?q=1')).toEqual({ url: 'https://example.com:8080/p?q=1' });
  });

  it('prefixes localhost and IP literals with http', () => {
    expect(resolveAddress('localhost')).toEqual({ url: 'http://localhost' });
    expect(resolveAddress('localhost:3000')).toEqual({ url: 'http://localhost:3000' });
    expect(resolveAddress('localhost:8080/path?q=1')).toEqual({ url: 'http://localhost:8080/path?q=1' });
    expect(resolveAddress('127.0.0.1:8000/x')).toEqual({ url: 'http://127.0.0.1:8000/x' });
    expect(resolveAddress('192.168.1.5:8000/path')).toEqual({ url: 'http://192.168.1.5:8000/path' });
    expect(resolveAddress('[::1]:3000')).toEqual({ url: 'http://[::1]:3000' });
  });

  it('rejects non-http(s) schemes like the backend does', () => {
    for (const bad of ['javascript:alert(1)', 'file:///c/', 'data:text/plain,hi', 'mailto:a@b.c', 'ftp://example.com']) {
      expect(resolveAddress(bad)).toEqual({ error: 'Only HTTP and HTTPS addresses can be opened.' });
    }
  });

  it('searches DuckDuckGo for free text and single words', () => {
    expect(resolveAddress('how do shells work')).toEqual({ url: 'https://duckduckgo.com/?q=how%20do%20shells%20work' });
    expect(resolveAddress('shells')).toEqual({ url: 'https://duckduckgo.com/?q=shells' });
  });

  it('rejects empty input', () => {
    expect(resolveAddress('')).toEqual({ error: 'Enter an address or search term.' });
    expect(resolveAddress('   ')).toEqual({ error: 'Enter an address or search term.' });
  });
  it('rejects addresses containing embedded credentials', () => {
    expect(resolveAddress('https://user:password@example.com')).toEqual({ error: 'Enter an address without embedded credentials.' });
  });
});

describe('BrowserPanel', () => {
  let rect: DOMRect;
  beforeEach(() => {
    vi.clearAllMocks();
    rect = new DOMRect(100, 150, 800, 500);
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => rect);
    vi.spyOn(HTMLElement.prototype, 'checkVisibility').mockReturnValue(true);
  });
  afterEach(() => vi.restoreAllMocks());

  async function typeAndSubmit(value: string) {
    const input = screen.getByLabelText('Address');
    fireEvent.change(input, { target: { value } });
    await act(async () => { fireEvent.submit(input.closest('form')!); });
  }

  it('navigates to a DuckDuckGo search when plain text is submitted', async () => {
    render(<BrowserPanel tabId="t1" onError={() => {}} />);
    await typeAndSubmit('how do shells work');
    expect(mocks.browserNavigate).toHaveBeenCalledWith('t1', 'https://duckduckgo.com/?q=how%20do%20shells%20work');
  });

  it('navigates localhost:3000 over http', async () => {
    render(<BrowserPanel tabId="t1" onError={() => {}} />);
    await typeAndSubmit('localhost:3000');
    expect(mocks.browserNavigate).toHaveBeenCalledWith('t1', 'http://localhost:3000');
  });

  it('reports a rejected scheme via onError and never navigates', async () => {
    const onError = vi.fn();
    render(<BrowserPanel tabId="t1" onError={onError} />);
    await typeAndSubmit('javascript:alert(1)');
    expect(onError).toHaveBeenCalledWith('Only HTTP and HTTPS addresses can be opened.');
    expect(mocks.browserNavigate).not.toHaveBeenCalled();
  });
  it('waits for the native surface before navigating', async () => {
    let show!: () => void;
    mocks.browserShow.mockImplementationOnce(() => new Promise<void>(resolve => { show = resolve; }));
    render(<BrowserPanel tabId="ready-test" onError={() => {}} />);
    await typeAndSubmit('https://example.com');
    expect(mocks.browserNavigate).not.toHaveBeenCalled();
    await act(async () => { show(); });
    await waitFor(() => expect(mocks.browserNavigate).toHaveBeenCalledWith('ready-test', 'https://example.com'));
  });
  it('closes after a pending show and drops navigation when its tab is closed', async () => {
    let show!: () => void;
    mocks.browserShow.mockImplementationOnce(() => new Promise<void>(resolve => { show = resolve; }));
    const view = render(<BrowserPanel tabId="closing-test" onError={() => {}} />);
    await typeAndSubmit('https://example.com');
    const closing = closeBrowserForTab('closing-test');
    view.unmount();
    expect(mocks.browserClose).not.toHaveBeenCalled();
    await act(async () => { show(); await closing; });
    expect(mocks.browserClose).toHaveBeenCalledWith('closing-test');
    expect(mocks.browserNavigate).not.toHaveBeenCalled();
  });
  it('updates position without a size change and avoids redundant native updates', async () => {
    render(<BrowserPanel tabId="moving-test" onError={() => {}} />);
    await typeAndSubmit('https://example.com');
    expect(mocks.browserShow).toHaveBeenLastCalledWith('moving-test', 100, 150, 800, 500);
    await act(async () => { window.dispatchEvent(new Event('resize')); });
    await new Promise(resolve => setTimeout(resolve, 40));
    expect(mocks.browserShow).toHaveBeenCalledTimes(1);
    rect = new DOMRect(200, 80, 800, 500);
    await act(async () => { window.dispatchEvent(new Event('scroll')); });
    await waitFor(() => expect(mocks.browserShow).toHaveBeenLastCalledWith('moving-test', 200, 80, 800, 500));
  });
  it('coalesces resize bursts while native layout is pending', async () => {
    let show!: () => void;
    mocks.browserShow.mockImplementationOnce(() => new Promise<void>(resolve => { show = resolve; }));
    render(<BrowserPanel tabId="burst-test" onError={() => {}} />);
    await typeAndSubmit('https://example.com');
    for (let i = 0; i < 20; i++) {
      rect = new DOMRect(i * 10, 80, 800, 500);
      window.dispatchEvent(new Event('resize'));
    }
    await new Promise(resolve => setTimeout(resolve, 40));
    expect(mocks.browserShow).toHaveBeenCalledTimes(1);
    await act(async () => { show(); });
    await waitFor(() => expect(mocks.browserNavigate).toHaveBeenCalledWith('burst-test', 'https://example.com'));
    expect(mocks.browserShow).toHaveBeenCalledTimes(2);
    expect(mocks.browserShow).toHaveBeenLastCalledWith('burst-test', 190, 80, 800, 500);
  });
  it('waits for a nonzero host before creating and navigating', async () => {
    rect = new DOMRect(0, 0, 0, 0);
    render(<BrowserPanel tabId="hidden-test" onError={() => {}} />);
    await typeAndSubmit('https://example.com');
    expect(mocks.browserShow).not.toHaveBeenCalled();
    expect(mocks.browserNavigate).not.toHaveBeenCalled();
    rect = new DOMRect(100, 150, 800, 500);
    window.dispatchEvent(new Event('resize'));
    await waitFor(() => expect(mocks.browserNavigate).toHaveBeenCalledWith('hidden-test', 'https://example.com'));
  });
  it('hides behind entering overlays even when their opacity starts at zero', async () => {
    render(<BrowserPanel tabId="overlay-test" onError={() => {}} />);
    await typeAndSubmit('https://example.com');
    const dialog = document.createElement('div');
    dialog.setAttribute('role', 'dialog'); dialog.style.opacity = '0';
    await act(async () => { document.body.append(dialog); });
    await waitFor(() => expect(mocks.browserHide).toHaveBeenCalledWith('overlay-test'));
    await act(async () => { dialog.remove(); });
    await waitFor(() => expect(mocks.browserShow).toHaveBeenCalledTimes(2));
  });
  it('hides a delayed old tab before showing the replacement tab', async () => {
    let show!: () => void;
    mocks.browserShow.mockImplementationOnce(() => new Promise<void>(resolve => { show = resolve; }));
    const view = render(<BrowserPanel key="old" tabId="old-tab" onError={() => {}} />);
    await typeAndSubmit('https://old.example');
    mocks.browserUrl.mockResolvedValueOnce({ url: 'https://new.example' });
    view.rerender(<BrowserPanel key="new" tabId="new-tab" onError={() => {}} />);
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 40)); });
    expect(mocks.browserShow).toHaveBeenCalledTimes(1);
    await act(async () => { show(); });
    await waitFor(() => expect(mocks.browserShow).toHaveBeenLastCalledWith('new-tab', 100, 150, 800, 500));
    expect(mocks.browserHide).toHaveBeenCalledWith('old-tab');
    expect(mocks.browserHide.mock.invocationCallOrder[0]).toBeLessThan(mocks.browserShow.mock.invocationCallOrder[1]);
    expect(mocks.browserNavigate).not.toHaveBeenCalled();
  });
});
