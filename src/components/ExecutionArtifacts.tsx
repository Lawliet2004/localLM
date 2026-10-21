import { ArtifactViewer } from './ArtifactViewer';
import { useState } from 'react';
import { File } from 'lucide-react';
import { api, errorMessage } from '../lib/api';

export function ExecutionArtifacts({ result }: { result: Record<string, unknown> }) {
  const [previews, setPreviews] = useState<Record<string, { loading?: boolean; error?: string; content?: string; mimeType?: string }>>({});
  const [showAll, setShowAll] = useState(false);
  const files = Array.isArray(result.artifacts) ? result.artifacts.filter((value): value is { path: string; sha256: string; bytes: number } =>
    value !== null && typeof value === 'object' && typeof value.path === 'string' && typeof value.sha256 === 'string' && /^[a-f0-9]{64}$/.test(value.sha256) && typeof value.bytes === 'number') : [];
  const errors = Array.isArray(result.artifactErrors) ? result.artifactErrors : [];
  async function preview(path: string, sha256: string) {
    if (typeof result.workspace !== 'string') return;
    setPreviews(current => ({ ...current, [path]: { loading: true } }));
    try { const value = await api.previewExecutionArtifact(result.workspace, path, sha256); setPreviews(current => ({ ...current, [path]: value })); }
    catch (e) { setPreviews(current => ({ ...current, [path]: { error: errorMessage(e) } })); }
  }
  if (!files.length && !errors.length) return null;
  return <section className="execution-artifacts" aria-label="Generated files"><h3>Generated files</h3>
    {(showAll ? files : files.slice(0, 16)).map(file => <div className="artifact-file" key={file.path}><span><File size={15} aria-hidden="true" /> {file.path} · {(file.bytes / 1024).toFixed(1)} KiB</span> <button className="secondary" aria-label={`Preview ${file.path}`} disabled={previews[file.path]?.loading} onClick={() => void preview(file.path, file.sha256)}>{previews[file.path]?.loading ? 'Loading…' : 'Preview file'}</button>
      {previews[file.path]?.error && <p role="alert">{previews[file.path].error}</p>}
      {previews[file.path]?.content !== undefined && <ArtifactViewer name={file.path} bytes={file.bytes} content={previews[file.path].content!} mimeType={previews[file.path].mimeType ?? 'text/plain'} onClose={() => setPreviews(current => ({ ...current, [file.path]: {} }))} />}
    </div>)}
    {files.length > 16 && <button className="secondary" onClick={() => setShowAll(!showAll)}>{showAll ? 'Show fewer files' : `Show all ${files.length} files`}</button>}
    {errors.map((value, index) => value && typeof value === 'object' && typeof value.path === 'string' && typeof value.error === 'string' ? <p key={index}>{value.path}: {value.error}</p> : null)}
  </section>;
}
