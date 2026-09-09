import { useEffect, useState } from 'react';
import { BookOpen, Search } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import catalog from '../lib/catalog.json';
import type { SkillDependencyStatus, SkillView } from '../lib/types';

const presets: SkillView[] = catalog.skills.map(item => ({ id: item.name, description: item.description, repo: item.url.replace('https://github.com/', ''), revision: '', sourcePath: item.path, files: [], installed: false, active: false }));
export function Skills() {
  const [items, setItems] = useState(presets);
  const [query, setQuery] = useState('');
  const [pending, setPending] = useState('');
  const [error, setError] = useState('');
  const [preview, setPreview] = useState<{ id: string; path: string; text: string } | null>(null);
  const [dependencies, setDependencies] = useState<{ id: string; rows: SkillDependencyStatus[] } | null>(null);
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.listSkills().then(value => { if (!disposed) setItems(value); }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, []);
  async function action(item: SkillView, kind: 'install' | 'remove' | 'activate') {
    setPending(item.id); setError('');
    try {
      if (kind === 'install') await api.installSkill(item.id);
      if (kind === 'remove') { await api.removeSkill(item.id); if (preview?.id === item.id) setPreview(null); if (dependencies?.id === item.id) setDependencies(null); }
      if (kind === 'activate') await api.setSkillActive(item.id, !item.active);
      setItems(await api.listSkills());
    } catch (e) { setError(errorMessage(e)); }
    finally { setPending(''); }
  }
  async function inspect(id: string, path: string) {
    setError('');
    try { setPreview({ id, path, text: await api.readSkillFile(id, path) }); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function checkDependencies(item: SkillView) {
    setPending(item.id); setError('');
    try { setDependencies({ id: item.id, rows: await api.skillDependencies(item.id) }); }
    catch (e) { setDependencies(null); setError(errorMessage(e)); }
    finally { setPending(''); }
  }
  const filtered = items.filter(item => `${item.id} ${item.description}`.toLowerCase().includes(query.toLowerCase()));
  return <div className="settings-page catalog-page"><div className="page-heading"><p className="eyebrow">EXTEND YOUR WORKSPACE</p><h1>Skills</h1><p>Reusable instructions and resources for specialized work.</p></div>
    <div className="catalog-toolbar"><label className="search-field"><Search size={16} /><input type="search" aria-label="Search skills" placeholder="Search skills…" value={query} onChange={event => setQuery(event.target.value)} /></label><span>{filtered.length} available presets</span></div>
    <p className="catalog-notice">Installed files are verified against pinned sources. Active skills guide your next messages and enable a tool to read their references and script source. Reading follows your conversation’s permissions; scripts need a separately enabled execution tool.</p>
    {error && <p className="error-banner" role="alert">{error}</p>}
    <div className="catalog-list">{filtered.map(item => <details className="catalog-item" key={item.id}><summary><span className="catalog-icon"><BookOpen size={18} /></span><span><strong>{item.id.split('-').map(word => word.charAt(0).toUpperCase() + word.slice(1)).join(' ')}</strong><small>{item.description}</small></span><small>{item.active ? 'Active' : item.installed ? 'Installed' : 'Not installed'}</small></summary><div className="catalog-detail">
      <span>Source repository</span><code>{item.repo}/{item.sourcePath}</code>{item.revision && <><span>Pinned revision</span><code>{item.revision}</code></>}
      <div className="connector-actions"><button className="primary" disabled={!nativeAvailable || !!pending} onClick={() => void action(item, item.installed ? 'activate' : 'install')}>{pending === item.id ? 'Working…' : item.installed ? item.active ? 'Deactivate' : 'Activate' : 'Install'}</button>
        {item.installed && <><button className="secondary" disabled={!!pending} onClick={() => void inspect(item.id, 'SKILL.md')}>Read instructions</button><button className="secondary" disabled={!!pending} onClick={() => void checkDependencies(item)}>Check dependencies</button><button className="secondary" disabled={!!pending} onClick={() => void action(item, 'install')}>Verify files</button><button className="secondary" disabled={!!pending} onClick={() => void action(item, 'remove')}>Remove</button></>}
      </div>
      {dependencies?.id === item.id && <div className="skill-dependencies"><strong>Dependencies</strong><ul>{dependencies.rows.map(row => <li key={`${row.dependency.kind}:${row.dependency.name}`}>{row.satisfied ? 'Ready' : 'Missing'} · {row.dependency.kind} · <code>{row.dependency.name}</code> — {row.dependency.detail} {row.remedy}</li>)}</ul><p><small>Checks only report status. Skill instructions never bypass conversation permissions; scripts run only through an enabled execution tool after approval.</small></p></div>}
      {item.installed && <label>Package files<select aria-label={`${item.id} package files`} value={preview?.id === item.id ? preview.path : ''} onChange={event => { if (event.target.value) void inspect(item.id, event.target.value); }}><option value="">Choose a file to inspect</option>{item.files.map(file => <option key={file.path} value={file.path}>{file.path}</option>)}</select></label>}
      {preview?.id === item.id && <div className="skill-preview"><strong>{preview.path}</strong><pre>{preview.text}</pre></div>}
    </div></details>)}</div>
    {!filtered.length && <div className="empty-state">No skills match “{query}”.</div>}
  </div>;
}
