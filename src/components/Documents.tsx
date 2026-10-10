import { useCallback, useEffect, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen, RefreshCw, Trash2 } from 'lucide-react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { DocCollection, DocHit } from '../lib/types';
import { SettingRow, SettingsGroup, SettingsHeader } from './Settings';

/** Private document collections: folders indexed locally and searchable by the
 *  agent through `docs_search`, which cites `path:line-line`. */
export function Documents() {
  const [collections, setCollections] = useState<DocCollection[]>([]);
  const [name, setName] = useState('');
  const [busy, setBusy] = useState(nativeAvailable);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [query, setQuery] = useState('');
  const [hits, setHits] = useState<DocHit[] | null>(null);
  const [searching, setSearching] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setCollections(await api.documentsListCollections());
      setError('');
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    if (!nativeAvailable) return;
    void refresh();
  }, [refresh]);

  const add = async () => {
    setBusy(true); setError(''); setNotice('');
    try {
      const root = await open({ directory: true, multiple: false, title: 'Choose a folder of documents' });
      if (typeof root !== 'string') return;
      const collection = await api.documentsAddCollection(root, name.trim());
      setName('');
      setNotice(`Indexed ${collection.fileCount} files (${collection.chunkCount} passages)${collection.error ? ` — ${collection.error}` : ''}.`);
      await refresh();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const act = async (action: () => Promise<unknown>, done: string) => {
    setBusy(true); setError(''); setNotice('');
    try {
      await action();
      setNotice(done);
      await refresh();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const search = async () => {
    setSearching(true); setError(''); setHits(null);
    try {
      setHits(await api.documentsSearch(query, null));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSearching(false);
    }
  };

  return (
    <div className="settings-page">
      <SettingsHeader title="Documents" description="Private folders the assistant can search. Everything is indexed and searched on this machine — nothing is uploaded." />
      <SettingsGroup title="Collections" description="Text documents only (Markdown, text, code — up to 2,000 files of 4 MiB each). PDFs and images are not indexed. The assistant cites file paths and line numbers.">
        <SettingRow label="New collection" stacked>
          <div className="execution-path">
            <input aria-label="Collection name" placeholder="Name (defaults to folder name)" maxLength={80} value={name} disabled={!nativeAvailable || busy} onChange={event => setName(event.target.value)} />
            <button type="button" className="secondary" disabled={!nativeAvailable || busy} onClick={() => void add()}>
              <FolderOpen size={13} aria-hidden="true" /> Add folder
            </button>
          </div>
        </SettingRow>
        {collections.length === 0 && !busy && <p className="settings-note">No collections yet. The assistant only sees folders you add here.</p>}
        {collections.map(collection => (
          <SettingRow
            key={collection.id}
            label={collection.name}
            description={<>
              <code>{collection.root}</code>
              <span className="settings-note"> · {collection.fileCount} files · {collection.chunkCount} passages{collection.indexedAt ? ` · indexed ${new Date(collection.indexedAt * 1000).toLocaleString()}` : ''}</span>
              {collection.error && <span className="setting-row-error"> {collection.error}</span>}
            </>}
          >
            <div className="execution-path">
              <button type="button" className="secondary" aria-label={`Reindex ${collection.name}`} disabled={busy} onClick={() => void act(() => api.documentsReindex(collection.id), `Reindexed ${collection.name}.`)}>
                <RefreshCw size={12} aria-hidden="true" />
              </button>
              <button type="button" className="secondary" aria-label={`Remove ${collection.name}`} disabled={busy} onClick={() => void act(() => api.documentsRemoveCollection(collection.id), `Removed ${collection.name}.`)}>
                <Trash2 size={12} aria-hidden="true" />
              </button>
            </div>
          </SettingRow>
        ))}
      </SettingsGroup>
      {collections.length > 0 && (
        <SettingsGroup title="Try the index" description="The same search the assistant's docs_search tool runs — returns the file and lines it would cite.">
          <SettingRow label="Search" stacked>
            <form className="execution-path" onSubmit={event => { event.preventDefault(); void search(); }}>
              <input aria-label="Search documents" placeholder="Search your collections…" maxLength={512} value={query} disabled={searching} onChange={event => setQuery(event.target.value)} />
              <button className="secondary" disabled={!query.trim() || searching}>{searching ? 'Searching…' : 'Search'}</button>
            </form>
          </SettingRow>
          {hits !== null && hits.length === 0 && <p className="settings-note">No matches.</p>}
          {hits?.map((hit, index) => (
            <SettingRow key={index} label={`${hit.path}:${hit.lineStart}–${hit.lineEnd}`} description={<>
              <span className="settings-note">{hit.collection}</span>
              <span className="doc-snippet">{hit.snippet}</span>
            </>} />
          ))}
        </SettingsGroup>
      )}
      {error && <p role="alert" className="error-banner">{error}</p>}
      {notice && <p role="status" className="success">{notice}</p>}
    </div>
  );
}
