import { useState } from 'react';
import { BookOpen, Search, Plug, ArrowUpRight } from 'lucide-react';
import catalog from '../lib/catalog.json';

export function Catalog({ kind }: { kind: 'connectors' | 'skills' }) {
  const [query, setQuery] = useState('');
  const items = catalog[kind].filter(item => `${item.name} ${item.description}`.toLowerCase().includes(query.toLowerCase()));
  return <div className="settings-page catalog-page"><div className="page-heading"><p className="eyebrow">EXTEND YOUR WORKSPACE</p><h1>{kind === 'connectors' ? 'Connectors' : 'Skills'}</h1><p>{kind === 'connectors' ? 'Bring your tools and information into the conversation.' : 'Give your assistant a way to approach specialized work.'}</p></div>
    <div className="catalog-toolbar"><label className="search-field"><Search size={16} /><input type="search" aria-label={`Search ${kind}`} placeholder={`Search ${kind}…`} value={query} onChange={e => setQuery(e.target.value)} /></label><span>{items.length} available presets</span></div>
    <p className="catalog-notice">TrueForge catalog · Browse the available presets. {kind === 'connectors' ? 'Account connection and tool execution are not enabled in this development build.' : 'Installation and activation are not enabled in this development build.'}</p>
    <div className="catalog-list">{items.map(item => <details className="catalog-item" key={item.name}><summary><span className="catalog-icon">{kind === 'connectors' ? <Plug size={18} /> : <BookOpen size={18} />}</span><span><strong>{item.name.split('-').map(word => word.charAt(0).toUpperCase() + word.slice(1)).join(' ')}</strong><small>{item.description}</small></span><ArrowUpRight size={15} /></summary><div className="catalog-detail"><span>{kind === 'connectors' ? 'MCP service' : 'Source repository'}</span><code>{item.url}</code>{'path' in item && <><span>Skill directory</span><code>{item.path}</code></>}</div></details>)}</div>
    {!items.length && <div className="empty-state">No {kind} match “{query}”.</div>}
  </div>;
}
