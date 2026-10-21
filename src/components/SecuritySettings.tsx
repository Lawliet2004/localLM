import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { AccessMode } from '../lib/types';
import { ApprovalSegmented, approvalNotes } from './ApprovalSegmented';

export function SecuritySettings({ accessMode, onAccessMode, busy, scope }: { accessMode: AccessMode; onAccessMode: (mode: AccessMode) => void; busy: boolean; scope?: string }) {
  const [offline, setOffline] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.getOfflineMode().then(value => { if (!disposed) { setOffline(value); setLoaded(true); } }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);
  async function saveOffline(value: boolean) {
    setSaving(true); setError(''); setNotice('');
    try { await api.setOfflineMode(value); setOffline(value); setNotice(value ? 'External research and weather retrieval disabled.' : 'External retrieval enabled for selected tools.'); }
    catch (e) { setError(errorMessage(e)); } finally { setSaving(false); }
  }
  const approvalDisabled = busy || !nativeAvailable;
  return <div className="settings-page">
    <div className="page-heading"><h1>Security</h1><p>Control tool approvals, external retrieval, and what stays on this device.</p></div>
    <section className="preference-section" aria-labelledby="security-approval-heading">
      <div className="preference-section-heading"><h2 id="security-approval-heading">Tool approval</h2><p>{scope ? `Applies to “${scope}” and new chats.` : 'Applies to new chats.'}</p></div>
      <div className="preference-group">
        <div className="preference-row preference-row-approve">
          <div className="preference-copy">
            <span className="preference-title" id="security-access-label">Approval policy</span>
            <span className="preference-desc">How much the assistant can do on its own.</span>
          </div>
          <ApprovalSegmented value={accessMode} onChange={onAccessMode} disabled={approvalDisabled} name="security-access" labelledBy="security-access-label" />
        </div>
        <div className="settings-note"><p>{approvalNotes[accessMode]} Bash and Python can require approval even in Full access. Browser and computer use start disabled and must be enabled per conversation.</p></div>
      </div>
    </section>
    <section className="preference-section" aria-labelledby="security-retrieval-heading">
      <div className="preference-section-heading"><h2 id="security-retrieval-heading">External retrieval</h2></div>
      <div className="preference-group">
        <div className="preference-row preference-toggle">
          <div className="preference-copy">
            <label htmlFor="security-offline">Offline research mode</label>
            <p id="security-offline-desc">Block new web research, page retrieval, and weather requests.</p>
          </div>
          <input id="security-offline" className="switch" type="checkbox" aria-describedby="security-offline-desc" checked={offline} disabled={!nativeAvailable || !loaded || busy || saving} onChange={event => void saveOffline(event.target.checked)} />
        </div>
        <div className="settings-note"><p>Saved research stays readable. This does not isolate shell or MCP networking, and does not block an explicitly selected API model.</p></div>
      </div>
    </section>
    <section className="preference-section" aria-labelledby="security-data-heading">
      <div className="preference-section-heading"><h2 id="security-data-heading">Data on this device</h2></div>
      <div className="preference-group">
        <div className="settings-note">
          <p>Conversations and settings stay on this device; the conversation database is not encrypted at rest. API credentials use the native credential vault.</p>
          <p>Local commands run with your account’s permissions — working-directory limits are not an operating-system sandbox.</p>
        </div>
      </div>
    </section>
    {error && <p className="error-banner" role="alert">{error}</p>}{notice && <p role="status">{notice}</p>}
  </div>;
}
