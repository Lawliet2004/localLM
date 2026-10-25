import { useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import { gib } from '../lib/runtimeGuidance';

export function ManagedStorage({ busy, onMoved }: { busy: boolean; onMoved?: () => Promise<void> }) {
  const [info, setInfo] = useState<{ root: string; availableBytes: number } | null>(null);
  const [moving, setMoving] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    void api.managedStorageInfo().then(value => { if (!disposed) setInfo(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);
  async function move() {
    const selected = await open({ directory: true, multiple: false, title: 'Choose an empty folder for models and runtimes' });
    if (typeof selected !== 'string') return;
    setMoving(true); setError(''); setNotice('');
    try { setInfo(await api.moveManagedStorage(selected)); setNotice('Storage location saved. The verified copy is ready; the original files are retained for recovery. Existing model settings continue to work.'); await onMoved?.(); }
    catch (e) { setError(errorMessage(e)); }
    finally { setMoving(false); }
  }
  return <section className="catalog-notice" aria-label="Managed storage">
    <h2>Model and runtime storage</h2>
    {info && <><p className="download-path">{info.root}</p><p>{gib(info.availableBytes)} available</p></>}
    <p>Unload the model and finish or cancel downloads before moving. Choose an empty folder with enough space for a verified copy. Keep the destination connected when using LocalLM.</p>
    <button type="button" className="secondary" disabled={!nativeAvailable || busy || moving} onClick={() => { void move().catch(e => setError(errorMessage(e))); }}>{moving ? 'Copying and verifying storage…' : 'Change storage location'}</button>
    {notice && <p role="status">{notice}</p>}{error && <p role="alert">{error}</p>}
  </section>;
}
