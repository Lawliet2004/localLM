import { useEffect, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { GitReview } from '../lib/types';

export function GitSettings({ root, onOpenReview }: { root?: string; onOpenReview: () => void }) {
  const [git, setGit] = useState<GitReview | null>(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    if (!nativeAvailable || !root) return;
    let disposed = false; setLoading(true); setError(''); setGit(null);
    api.workspaceGit(undefined, root).then(value => { if (!disposed) setGit(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [root]);
  const status = !root ? 'Choose a workspace to inspect a repository.'
    : loading ? 'Inspecting repository…'
    : git ? (git.repository === false ? 'This workspace is not a Git repository.' : `${git.branch || 'No current branch'} · ${git.files?.length || 0} changed files`)
    : '';
  return <div className="settings-page"><div className="page-heading"><h1>Git</h1><p>Review local changes, stage files, and manage commits.</p></div>
    {!nativeAvailable ? <p className="catalog-notice">Git operations are available in the desktop application.</p> : <>
      <section className="preference-section" aria-labelledby="git-repo-heading">
        <div className="preference-section-heading"><h2 id="git-repo-heading">Repository</h2></div>
        <div className="preference-group">
          <div className="preference-row"><div className="preference-copy">
            <span className="preference-title">Current repository</span>
            <span className="preference-desc">{root || 'No workspace selected.'}{status && ` — ${status}`}</span>
          </div></div>
          <button type="button" className="preference-row preference-nav" aria-labelledby="git-review-title" aria-describedby="git-review-desc" onClick={onOpenReview}>
            <span className="preference-copy">
              <span className="preference-title" id="git-review-title">Open repository review</span>
              <span className="preference-desc" id="git-review-desc">Stage, commit, and switch branches.</span>
            </span>
            <ChevronRight size={17} className="preference-nav-chevron" />
          </button>
        </div>
      </section>
      <section className="preference-section" aria-labelledby="git-safe-heading">
        <div className="preference-section-heading"><h2 id="git-safe-heading">What’s included</h2></div>
        <div className="preference-group"><div className="settings-note">
          <p>Review shows index and working-tree status, text diffs, and recent commits. Stage or unstage files, create or switch branches without forcing a checkout, and commit with confirmation.</p>
          <p>Remote fetch, pull, and push stay available through approved shell commands with your Git credentials.</p>
        </div></div>
      </section>
    </>}
    {error && <p className="error-banner" role="alert">{error}</p>}
  </div>;
}
