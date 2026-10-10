import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { BrowserPanel, resolveAddress } from './BrowserPanel';

const mocks = vi.hoisted(() => ({
  browserShow: vi.fn(() => Promise.resolve()),
  browserHide: vi.fn(() => Promise.resolve()),
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
});

describe('BrowserPanel', () => {
  beforeEach(() => vi.clearAllMocks());

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

  it('waits for the native browser surface before navigating and repositions it after the panel moves', async () => {
    const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ left: 300, top: 80, width: 400, height: 500 } as DOMRect);
    let finishShow!: () => void;
    mocks.browserShow.mockImplementationOnce(() => new Promise<void>(resolve => { finishShow = resolve; }));
    try {
      const { container } = render(<div className="workspace"><div className="workspace-panel"><BrowserPanel tabId="t1" onError={() => {}} /></div></div>);
      await typeAndSubmit('example.com');
      expect(mocks.browserNavigate).not.toHaveBeenCalled();
      await act(async () => finishShow());
      await waitFor(() => expect(mocks.browserNavigate).toHaveBeenCalledWith('t1', 'https://example.com'));
      fireEvent.animationEnd(container.querySelector('.workspace-panel')!);
      await waitFor(() => expect(mocks.browserShow).toHaveBeenCalledTimes(2));
    } finally {
      rect.mockRestore();
    }
  });

  it('hides a browser that finishes showing after its panel unmounts', async () => {
    const rect = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ left: 300, top: 80, width: 400, height: 500 } as DOMRect);
    let finishShow!: () => void;
    const calls: string[] = [];
    mocks.browserShow.mockImplementationOnce(() => {
      calls.push('show');
      return new Promise<void>(resolve => { finishShow = resolve; });
    });
    mocks.browserHide.mockImplementationOnce(() => {
      calls.push('hide');
      return Promise.resolve();
    });
    try {
      const panel = render(<BrowserPanel tabId="late-show" onError={() => {}} />);
      await waitFor(() => expect(mocks.browserShow).toHaveBeenCalledTimes(1));
      panel.unmount();
      expect(mocks.browserHide).not.toHaveBeenCalled();
      await act(async () => finishShow());
      await waitFor(() => expect(mocks.browserHide).toHaveBeenCalledWith('late-show'));
      expect(calls).toEqual(['show', 'hide']);
    } finally {
      rect.mockRestore();
    }
  });
});
