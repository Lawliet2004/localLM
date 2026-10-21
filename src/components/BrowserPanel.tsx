import { useLayoutEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ArrowLeft, ArrowRight, ExternalLink, Globe, RotateCw } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';

// Search engine for address-bar queries: DuckDuckGo (privacy-friendly default,
// consistent with the app's self-hosted/privacy lean).
const SEARCH_URL = 'https://duckduckgo.com/?q=';
// Tabs share one native layer. Serialize across tabs as well, so a delayed
// show from the previous tab finishes and hides before the next tab appears.
let viewOperation = Promise.resolve();
function enqueueView(action: () => Promise<void>) {
  const next = viewOperation.catch(() => {}).then(action);
  viewOperation = next;
  return next;
}
export function closeBrowserForTab(tabId: string) { return enqueueView(() => api.browserClose(tabId)); }

// ponytail: heuristic classifier for UX only — checked_url in inspector_browser.rs
// is the real http/https boundary, so the loose host regexes below (no octet
// range check, underscores allowed) are fine: bad hosts still get rejected
// server-side and surface through onError.
export function resolveAddress(raw: string): { url: string } | { error: string } {
  const trimmed = raw.trim();
  if (!trimmed) return { error: 'Enter an address or search term.' };
  if (/^https?:\/\//i.test(trimmed)) {
    try {
      const url = new URL(trimmed);
      if (!url.hostname || url.username || url.password) return { error: 'Enter an address without embedded credentials.' };
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
  const [working, setWorking] = useState(false);
  const readyRef = useRef<(() => Promise<void>) | null>(null);
  const liveRef = useRef(true);

  useLayoutEffect(() => {
    if (!nativeAvailable || !hostRef.current) return;
    const host = hostRef.current;
    liveRef.current = true;
    let cancelled = false;
    let requested = false;
    let overlayed = false;
    let frame: number | null = null;
    let pending: Promise<void> | null = null;
    let dirty = false;
    let shown = false;
    let created = false;
    let bounds: number[] | null = null;
    const readyWaiters: { resolve: () => void; reject: (error: unknown) => void }[] = [];
    const report = (error: unknown) => {
      readyWaiters.splice(0).forEach(waiter => waiter.reject(error));
      if (!cancelled) errorRef.current(errorMessage(error));
    };
    // Native surface changes are ordered: navigation waits for creation, and
    // a resize cannot reveal a page over a menu or a modal.
    const enqueue = (action: () => Promise<void>) => {
      return enqueueView(action);
    };
    const layout = (): Promise<void> => {
      dirty = true;
      if (pending) return pending;
      // Keep at most one layout operation queued. Read geometry when that
      // operation runs, so slow native IPC never replays obsolete frames.
      const next = enqueue(async () => {
        while (dirty && !cancelled && requested) {
          dirty = false;
          const rect = host.getBoundingClientRect();
          const visible = host.checkVisibility?.({ checkVisibilityCSS: true }) ?? host.getClientRects().length > 0;
          // Native content cannot participate in ancestor CSS animations.
          // Hide it during any such motion and restore at the final bounds.
          const animations: Animation[] = [];
          for (let el: HTMLElement | null = host; el; el = el.parentElement) {
            animations.push(...(el.getAnimations?.() ?? []).filter(animation => animation.playState === 'running' || animation.pending));
          }
          const hidden = overlayed || !visible || rect.width < 1 || rect.height < 1 || animations.length > 0;
          if (hidden) {
            if (shown) { await api.browserHide(tabId); shown = false; bounds = null; }
            // Poll only while the host's ancestors are moving. This also
            // handles animations already running when the tab mounts.
            if (animations.length) resized();
            continue;
          }
          const nextBounds = [rect.left, rect.top, rect.width, rect.height].map(Math.round);
          if (shown && bounds?.every((value, index) => value === nextBounds[index])) continue;
          await api.browserShow(tabId, ...nextBounds as [number, number, number, number]);
          shown = true;
          created = true;
          bounds = nextBounds;
          readyWaiters.splice(0).forEach(waiter => waiter.resolve());
        }
      });
      pending = next;
      const complete = () => { if (pending === next) pending = null; };
      void next.then(complete, complete);
      return next;
    };
    readyRef.current = async () => {
      requested = true;
      await layout();
      if (!cancelled && !created) await new Promise<void>((resolve, reject) => readyWaiters.push({ resolve, reject }));
    };
    const resized = () => {
      if (cancelled || frame !== null) return;
      frame = requestAnimationFrame(() => {
        frame = null;
        void layout().catch(report);
      });
    };
    void api.browserUrl(tabId).then(({ url }) => {
      if (cancelled || requested) return;
      if (url && url !== 'about:blank') {
        setCurrent(url); setAddress(url); requested = true; resized();
      }
    }).catch(e => { if (!cancelled) errorRef.current(errorMessage(e)); });

    const observer = new ResizeObserver(resized);
    observer.observe(host);
    window.addEventListener('resize', resized);
    window.addEventListener('scroll', resized, true);
    const motion = (event: Event) => {
      if (event.target instanceof Element && event.target.contains(host)) resized();
    };
    const motionEvents = ['transitionrun', 'transitionend', 'transitioncancel', 'animationstart', 'animationend', 'animationcancel'];
    motionEvents.forEach(name => document.addEventListener(name, motion, true));
    const updateVisibility = () => {
      // An entering menu can start at opacity zero and still blocks native
      // content. Do not use opacity to decide whether it needs protection.
      const next = [...document.querySelectorAll<HTMLElement>('.app-menu[open], [role="dialog"]')]
        .some(el => el.checkVisibility?.({ checkVisibilityCSS: true }) ?? el.getClientRects().length > 0);
      if (next !== overlayed) {
        overlayed = next;
        void layout().catch(report);
      }
      // Class/style and DOM changes can move the host without resizing it
      // (sidebar collapse, panel expansion, toolbars, error messages).
      resized();
    };
    const overlays = new MutationObserver(updateVisibility);
    overlays.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['open', 'hidden', 'class', 'style'] });
    updateVisibility();

    const unlisten = listen<{ label: string; url: string }>('browser-navigated', event => {
      if (cancelled || event.payload.label !== `inspector-browser-${tabId}`) return;
      setCurrent(event.payload.url);
      setAddress(event.payload.url);
      navigateRef.current?.(event.payload.url);
    }).catch(e => { if (!cancelled) errorRef.current(errorMessage(e)); return () => {}; });

    return () => {
      cancelled = true; liveRef.current = false; readyRef.current = null;
      readyWaiters.splice(0).forEach(waiter => waiter.resolve());
      observer.disconnect();
      overlays.disconnect();
      if (frame !== null) cancelAnimationFrame(frame);
      window.removeEventListener('resize', resized);
      window.removeEventListener('scroll', resized, true);
      motionEvents.forEach(name => document.removeEventListener(name, motion, true));
      void unlisten.then(stop => stop());
      void enqueue(() => api.browserHide(tabId)).catch(() => {});
    };
  }, [tabId]);

  async function navigate() {
    const resolved = resolveAddress(address);
    if ('error' in resolved) { onError(resolved.error); return; }
    setWorking(true);
    try {
      await readyRef.current?.();
      if (!liveRef.current) return;
      await enqueueView(() => api.browserNavigate(tabId, resolved.url));
      if (!liveRef.current) return;
      setCurrent(resolved.url); setAddress(resolved.url);
      onNavigate?.(resolved.url);
    } catch (e) { if (liveRef.current) onError(errorMessage(e)); }
    finally { if (liveRef.current) setWorking(false); }
  }

  function run(action: Promise<unknown>) { void action.catch(e => onError(errorMessage(e))); }

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
      <button className="icon-button" aria-label="Back" disabled={!current} onClick={() => run(api.browserGoBack(tabId))}><ArrowLeft size={15} /></button>
      <button className="icon-button" aria-label="Forward" disabled={!current} onClick={() => run(api.browserGoForward(tabId))}><ArrowRight size={15} /></button>
      <button className="icon-button" aria-label="Reload" disabled={!current} onClick={() => run(api.browserReload(tabId))}><RotateCw size={15} /></button>
      <form onSubmit={event => { event.preventDefault(); void navigate(); }}>
        <input aria-label="Address" placeholder="Search or enter a URL" value={address} onChange={event => setAddress(event.target.value)} required /><button className="file-open" disabled={working}>{working ? 'Opening…' : 'Go'}</button>
      </form>
      <button className="icon-button" aria-label="Open in system browser" disabled={!current} onClick={() => run(openUrl(current))}><ExternalLink size={15} /></button>
    </div>
    <div className="browser-host" ref={hostRef}>{!current && <div className="browser-start"><Globe size={32} /><h2>Browse the web</h2><p>Enter a website, localhost address, or search above.</p></div>}</div>
  </div>;
}
