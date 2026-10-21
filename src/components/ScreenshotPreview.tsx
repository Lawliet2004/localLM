import { ArtifactViewer } from './ArtifactViewer';
import { useEffect, useState } from 'react';
import { api, errorMessage } from '../lib/api';

export function ScreenshotPreview({ id, conversationId }: { id: string; conversationId: string }) {
  const [expanded, setExpanded] = useState(false);
  const [url, setUrl] = useState('');
  const [error, setError] = useState('');
  useEffect(() => {
    if (!expanded || url) return;
    let disposed = false;
    api.getArtifact(id).then(artifact => {
      if (disposed) return;
      if (!artifact || artifact.conversationId !== conversationId || artifact.mimeType !== 'image/png' || !artifact.content.startsWith('data:image/png;base64,')) throw new Error('Screenshot is unavailable in this conversation.');
      setUrl(artifact.content);
    }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, [expanded, id, conversationId, url]);
  return <details className="automation-screenshot" onToggle={e => setExpanded(e.currentTarget.open)}><summary>View screenshot</summary>{error ? <p role="alert">{error}</p> : url ? <ArtifactViewer name="Automation screenshot" content={url} mimeType="image/png" /> : <p>Loading screenshot…</p>}</details>;
}
