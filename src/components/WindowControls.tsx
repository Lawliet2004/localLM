import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { nativeAvailable, errorMessage } from '../lib/api';

export function WindowControls({ onError }: { onError: (message: string) => void }) {
  const [maximized, setMaximized] = useState(false);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const window = getCurrentWindow();
    const sync = () => { void window.isMaximized().then(value => { if (!disposed) setMaximized(value); }).catch(() => {}); };
    sync();
    void window.onResized(sync).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  }, []);
  async function act(action: 'minimize' | 'toggleMaximize' | 'close') {
    try { await getCurrentWindow()[action](); } catch (error) { onError(errorMessage(error)); }
  }
  return <>
    <div className="titlebar-drag-region" data-tauri-drag-region aria-hidden="true" onDoubleClick={() => { if (nativeAvailable) void act('toggleMaximize'); }} />
    <div className="window-controls">
      <button aria-label="Minimize window" title="Minimize" disabled={!nativeAvailable} onClick={() => void act('minimize')}><svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><path d="M1 6.5h10" /></svg></button>
      <button aria-label={maximized ? 'Restore window' : 'Maximize window'} title={maximized ? 'Restore' : 'Maximize'} disabled={!nativeAvailable} onClick={() => void act('toggleMaximize')}><svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">{maximized ? <path d="M3.5 3.5v-2h7v7h-2M1.5 3.5h7v7h-7z" /> : <rect x="1.5" y="1.5" width="9" height="9" />}</svg></button>
      <button className="window-close" aria-label="Close window" title="Close" disabled={!nativeAvailable} onClick={() => void act('close')}><svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true"><path d="m1 1 10 10M11 1 1 11" /></svg></button>
    </div>
  </>;
}
