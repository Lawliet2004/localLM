import { useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ArrowLeft, ArrowRight, ExternalLink, Globe, RotateCw } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';

// Search engine for address-bar queries: DuckDuckGo (privacy-friendly default,
// consistent with the app's self-hosted/privacy lean).
const SEARCH_URL = 'https://duckduckgo.com/?q=';
const browserOperations = new Map<string, Promise<void>>();

function queueBrowserOperation(tabId: string, operation: () => Promise<void>) {
  const previous = browserOperations.get(tabId) ?? Promise.resolve();
  const next = previous.catch(() => {}).then(operation);
  browserOperations.set(tabId, next);
  void next.finally(() => { if (browserOperations.get(tabId) === next) browserOperations.delete(tabId); }).catch(() => {});
  return next;
}

export function closeBrowserTab(tabId: string) {
  return queueBrowserOperation(tabId, () => api.browserClose(tabId));
}

// ponytail: heuristic classifier for UX only — checked_url in inspector_browser.rs
// is the real http/https boundary, so the loose host regexes below (no octet
// range check, underscores allowed) are fine: bad hosts still get rejected
// server-side and surface through onError.
export function resolveAddress(raw: string): { url: string } | { error: string } {
  const trimmed = raw.trim();
  if (!trimmed) return { error: 'Enter an address or search term.' };
  if (/^https?:\/\//i.test(trimmed)) {
    try {
      new URL(trimmed);
      return { url: trimmed };
    } catch {
      return { error: 'Enter a valid address.' };
    }
  }
  // Host[:port][/path] shapes run before the scheme rule: 'localhost:3000' and
  // 'example.com:8080' would otherwise look like schemes. Dev convention —
  // loopback/LAN addresses get http, real domains get https.
  if (/^localhost(:\d{1,5})?([/?#].*)?$/i.test(trimmed)
    || /^\d{1,3}(\.\d{1,3}){3}(:\d{1,5})?([/?#].*)?$/.test(trimmed)
    || /^\[[0-9a-fA-F:]+\](:\d{1,5})?([/?#].*)?$/.test(trimmed)) return { url: `http://${trimmed}` };
  if (/^[\w-]+(\.[\w-]+)*\.[a-zA-Z]{2,}(:\d{1,5})?([/?#].*)?$/.test(trimmed)) return { url: `https://${trimmed}` };
  if (trimmed === 'about:blank') return { url: trimmed };
  if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(trimmed)) return { error: 'Only HTTP and HTTPS addresses can be opened.' };
  return { url: `${SEARCH_URL}${encodeURIComponent(trimmed)}` };
}

export function BrowserPanel({ tabId, onNavigate, onError }: { tabId: string; onNavigate?: (url: string) => void; onError: (message: string) => void }) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const navigateRef = useRef(onNavigate);
  const errorRef = useRef(onError);
  navigateRef.current = onNavigate;
  errorRef.current = onError;
  const [address, setAddress] = useState('');
  const [current, setCurrent] = useState('');

  useEffect(() => {
    if (!nativeAvailable || !hostRef.current) return;
    const host = hostRef.current;
    let disposed = false;
    let hidden = false;
    // The child webview is a native surface overlaid on this rect: keep it
    // glued to the host through every layout change.
    const layout = () => {
      if (disposed || hidden) return;
      const rect = host.getBoundingClientRect();
      if (rect.width < 1 || rect.height < 1) return;
      void queueBrowserOperation(tabId, () => api.browserShow(tabId, rect.left, rect.top, rect.width, rect.height))
        .catch(e => errorRef.current(errorMessage(e)));
    };
    layout();
    void api.browserUrl(tabId).then(({ url }) => {
      if (url && url !== 'about:blank') { setCurrent(url); setAddress(url); }
    }).catch(() => {});

    const observer = new ResizeObserver(layout);
    observer.observe(host);
    const workspace = host.closest('.workspace');
    if (workspace) observer.observe(workspace);
    const panel = host.closest('.workspace-panel');
    const onPanelMotion = (event: Event) => { if (event.target === panel) layout(); };
    panel?.addEventListener('animationend', onPanelMotion);
    panel?.addEventListener('transitionend', onPanelMotion);
    window.addEventListener('resize', layout);

    // The overlay floats above the DOM and would swallow open menus and modal
    // dialogs; hide it while a *visible* one is up and restore once it closes.
    // CSS-only popovers (e.g. context-popover) stay in the DOM permanently, so
    // a bare presence check would keep the webview hidden forever.
    const updateVisibility = () => {
      const overlayed = [...document.querySelectorAll<HTMLElement>('.app-menu[open], [role="dialog"]')]
        .some(el => el.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true }));
      if (overlayed === hidden) return;
      hidden = overlayed;
      if (hidden) void queueBrowserOperation(tabId, () => api.browserHide(tabId)).catch(() => {});
      else layout();
    };
    const overlays = new MutationObserver(updateVisibility);
    overlays.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['open', 'hidden', 'class', 'style'] });

    const unlisten = listen<{ label: string; url: string }>('browser-navigated', event => {
      if (event.payload.label !== `inspector-browser-${tabId}`) return;
      setCurrent(event.payload.url);
      setAddress(event.payload.url);
      navigateRef.current?.(event.payload.url);
    });

    return () => {
      disposed = true;
      observer.disconnect();
      overlays.disconnect();
      panel?.removeEventListener('animationend', onPanelMotion);
      panel?.removeEventListener('transitionend', onPanelMotion);
      window.removeEventListener('resize', layout);
      void unlisten.then(stop => stop());
      void queueBrowserOperation(tabId, () => api.browserHide(tabId)).catch(() => {});
    };
  }, [tabId]);

  function navigate() {
    const resolved = resolveAddress(address);
    if ('error' in resolved) { onError(resolved.error); return; }
    setCurrent(resolved.url);
    setAddress(resolved.url);
    void queueBrowserOperation(tabId, () => api.browserNavigate(tabId, resolved.url)).catch(e => onError(errorMessage(e)));
  }

  if (!nativeAvailable) {
    return <form className="panel-browser" onSubmit={event => {
      event.preventDefault();
      const resolved = resolveAddress(address);
      if ('error' in resolved) { onError(resolved.error); return; }
      window.open(resolved.url, '_blank', 'noopener,noreferrer');
    }}>
      <Globe size={28} /><h2>Open a website</h2><p className="muted">The embedded browser needs the desktop application; preview opens a new window instead.</p>
      <label htmlFor="panel-browser-url">Website address</label>
      <input id="panel-browser-url" placeholder="Search or enter a URL" value={address} onChange={event => setAddress(event.target.value)} required />
      <button className="primary">Open in browser</button>
    </form>;
  }

  return <div className="browser-shell">
    <div className="browser-toolbar">
      <button className="icon-button" aria-label="Back" disabled={!current} onClick={() => void api.browserGoBack(tabId).catch(() => {})}><ArrowLeft size={15} /></button>
      <button className="icon-button" aria-label="Forward" disabled={!current} onClick={() => void api.browserGoForward(tabId).catch(() => {})}><ArrowRight size={15} /></button>
      <button className="icon-button" aria-label="Reload" disabled={!current} onClick={() => void api.browserReload(tabId).catch(() => {})}><RotateCw size={15} /></button>
      <form onSubmit={event => { event.preventDefault(); navigate(); }}>
        <input aria-label="Address" placeholder="Search or enter a URL" value={address} onChange={event => setAddress(event.target.value)} required />
      </form>
      <button className="icon-button" aria-label="Open in system browser" disabled={!current} onClick={() => void openUrl(current).catch(() => {})}><ExternalLink size={15} /></button>
    </div>
    <div className="browser-host" ref={hostRef} />
  </div>;
}
