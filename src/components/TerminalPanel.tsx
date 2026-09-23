import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { RotateCcw } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import '@xterm/xterm/css/xterm.css';

// One PTY session per workspace tab, keyed by tab id. Sessions outlive the
// component: switching tabs detaches the UI, returning replays scrollback.
// A pending spawn is shared so StrictMode's double-mount opens one shell.
const sessionsByTab = new Map<string, string | Promise<string>>();

function ensureSession(tabKey: string, cols: number, rows: number): Promise<string> {
  const existing = sessionsByTab.get(tabKey);
  if (existing) return Promise.resolve(existing);
  const spawn = api.terminalOpen(cols, rows)
    // The tab may have closed (or respawned) while this spawn was in
    // flight; only a still-current entry may be replaced or removed.
    .then(result => { if (sessionsByTab.get(tabKey) === spawn) sessionsByTab.set(tabKey, result.id); return result.id; })
    .catch(e => { if (sessionsByTab.get(tabKey) === spawn) sessionsByTab.delete(tabKey); throw e; });
  sessionsByTab.set(tabKey, spawn);
  return spawn;
}

/** Kill the shell owned by a closed workspace tab, if one was started. */
export function closeTerminalForTab(tabKey: string) {
  const entry = sessionsByTab.get(tabKey);
  sessionsByTab.delete(tabKey);
  // A pending spawn still resolves to a shell; kill it once it lands.
  if (entry) void Promise.resolve(entry).then(id => api.terminalClose(id)).catch(() => {});
}

export function TerminalPanel({ tabKey }: { tabKey: string }) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const liveIdRef = useRef<string | null>(null);
  const [exited, setExited] = useState(false);
  const [error, setError] = useState('');
  const [nonce, setNonce] = useState(0);

  useEffect(() => {
    if (!nativeAvailable || !hostRef.current) return;
    const host = hostRef.current;
    const styles = getComputedStyle(document.documentElement);
    const token = (name: string, fallback: string) => styles.getPropertyValue(name).trim() || fallback;
    const term = new Terminal({
      cursorBlink: true,
      fontFamily: '"Cascadia Mono", "Cascadia Code", Consolas, monospace',
      fontSize: 12.5,
      scrollback: 4000,
      theme: {
        background: token('--background', '#191a1b'),
        foreground: token('--text', '#ecedf2'),
        cursor: token('--accent-strong', '#6ee7b7'),
        selectionBackground: token('--hover', '#20222b'),
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    fit.fit();
    setError('');

    let cancelled = false;
    const attach = async (retry: boolean): Promise<void> => {
      let id: string;
      try {
        id = await ensureSession(tabKey, term.cols, term.rows);
      } catch (e) {
        if (!cancelled) setError(errorMessage(e));
        return;
      }
      if (cancelled) return;
      try {
        const result = await api.terminalAttach(id, event => {
          // The backend keeps routing events to this channel until the
          // session dies or is re-attached; drop them once torn down.
          if (cancelled) return;
          if (event.type === 'output' && event.data) term.write(event.data);
          else if (event.type === 'exit') {
            liveIdRef.current = null;
            setExited(true);
          }
        });
        if (cancelled) return;
        liveIdRef.current = result.id;
        setExited(result.exited);
      } catch (e) {
        // A remembered id can point at a shell killed while the tab was
        // inactive; drop it and respawn once.
        if (retry) {
          if (sessionsByTab.get(tabKey) === id) sessionsByTab.delete(tabKey);
          return attach(false);
        }
        if (!cancelled) setError(errorMessage(e));
      }
    };
    void attach(true);

    const dataSub = term.onData(data => {
      const id = liveIdRef.current;
      if (id) void api.terminalInput(id, data).catch(() => {});
    });
    const observer = new ResizeObserver(() => {
      try {
        fit.fit();
      } catch {
        return;
      }
      const id = liveIdRef.current;
      if (id) void api.terminalResize(id, term.cols, term.rows).catch(() => {});
    });
    observer.observe(host);
    return () => {
      cancelled = true;
      liveIdRef.current = null;
      dataSub.dispose();
      observer.disconnect();
      term.dispose();
    };
    // nonce restarts the shell; a tabKey change re-attaches the selected tab.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nonce, tabKey]);

  function restart() {
    liveIdRef.current = null;
    setExited(false);
    closeTerminalForTab(tabKey);
    setNonce(n => n + 1);
  }

  if (!nativeAvailable) {
    return <p className="panel-empty">The terminal runs a real shell and needs the desktop application.</p>;
  }

  return <div className="terminal-shell">
    <div className="terminal-host" ref={hostRef} />
    {(exited || error) && <div className="terminal-status">
      <span>{error || 'Shell exited.'}</span>
      <button type="button" onClick={restart}><RotateCcw size={13} /> New shell</button>
    </div>}
  </div>;
}
