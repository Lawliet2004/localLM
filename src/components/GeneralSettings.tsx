import { ProjectDocuments } from './ProjectDocuments';
import { ChevronRight, ShieldCheck } from 'lucide-react';
import type { AccessMode } from '../lib/types';
import type { Page } from './Sidebar';
import { ApprovalSegmented, approvalNotes } from './ApprovalSegmented';

export function GeneralSettings({ accessMode, onAccessMode, busy, nativeAvailable, projectPath, modelLabel, scopeLabel, onPage }: {
  accessMode: AccessMode; onAccessMode: (mode: AccessMode) => void;
  busy: boolean; nativeAvailable: boolean; projectPath?: string; modelLabel: string; scopeLabel?: string; onPage: (page: Page) => void;
}) {
  const disabled = busy || !nativeAvailable;
  return <div className="settings-page general-settings">
    <div className="page-heading"><h1>General</h1><p>Approvals and workspace essentials.</p></div>
    <section className="preference-section" aria-labelledby="permissions-heading">
      <div className="preference-section-heading"><h2 id="permissions-heading">Permissions</h2></div>
      <div className="preference-group">
        <div className="preference-row preference-row-approve">
          <div className="preference-copy">
            <span className="preference-title" id="general-access-label">Tool approval</span>
            <span className="preference-desc">How much the assistant can do on its own.</span>
          </div>
          <ApprovalSegmented value={accessMode} onChange={onAccessMode} disabled={disabled} name="general-access" labelledBy="general-access-label" />
        </div>
        <button type="button" className="preference-row preference-nav" aria-labelledby="general-tools-title" aria-describedby="general-tools-desc" onClick={() => onPage('tools')}>
          <span className="preference-copy">
            <span className="preference-title" id="general-tools-title">Tools &amp; permissions</span>
            <span className="preference-desc" id="general-tools-desc">Choose what the assistant can use.</span>
          </span>
          <ChevronRight size={17} className="preference-nav-chevron" />
        </button>
      </div>
      <p className={`preference-footnote ${accessMode === 'fullAccess' ? 'permission-caution' : ''}`}><ShieldCheck size={14} />{approvalNotes[accessMode]} {scopeLabel ? `Applies to “${scopeLabel}” and new chats.` : 'Applies to new chats.'}</p>
    </section>
    <section className="preference-section" aria-labelledby="workspace-heading">
      <div className="preference-section-heading"><h2 id="workspace-heading">Workspace</h2></div>
      <div className="preference-group">
        <button type="button" className="preference-row preference-nav" aria-labelledby="general-project-title" aria-describedby="general-project-desc" onClick={() => onPage('chat')}>
          <span className="preference-copy">
            <span className="preference-title" id="general-project-title">Project folder</span>
            <span className="preference-desc" id="general-project-desc">{projectPath || 'Choose a folder to work in.'}</span>
          </span>
          <ChevronRight size={17} className="preference-nav-chevron" />
        </button>
        <button type="button" className="preference-row preference-nav" aria-labelledby="general-models-title" aria-describedby="general-models-desc" onClick={() => onPage('models')}>
          <span className="preference-copy">
            <span className="preference-title" id="general-models-title">Models &amp; runtime</span>
            <span className="preference-desc" id="general-models-desc">{modelLabel === 'Local model' ? 'Choose a model.' : modelLabel}</span>
          </span>
          <ChevronRight size={17} className="preference-nav-chevron" />
        </button>
      </div>
    </section>
    <ProjectDocuments root={projectPath} busy={busy} />
    <footer className="general-about">
      <span className="general-wordmark">local<span>LM</span><span className="wordmark-dot" /></span>
      <span className="environment-badge"><span className="local-indicator" />{nativeAvailable ? 'Desktop application' : 'Browser preview'}</span>
    </footer>
  </div>;
}
