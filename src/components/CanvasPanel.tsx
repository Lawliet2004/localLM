// Right-side canvas: a resizable panel rendering the selected artifact
// (OpenUI program, HTML or code) with a Source view and version history.
// Later blocks sharing an artifact title supersede earlier versions.
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from 'react';
import { Check, ChevronLeft, ChevronRight, Copy, X } from 'lucide-react';
import type { CanvasArtifact } from '../lib/canvas';

const GenUiSurface = lazy(() => import('../genui/GenUiSurface').then(module => ({ default: module.GenUiSurface })));

const MIN_WIDTH = 300;
const MAX_WIDTH = 760;
const WIDTH_KEY = 'locallm-canvas-width';

function clampWidth(width: number) {
  return Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, Math.round(width)));
}

export function CanvasPanel({ artifact, versionCount, onVersion, onClose, onFollowUp }: {
  artifact: CanvasArtifact;
  versionCount: number;
  onVersion: (version: number) => void;
  onClose: () => void;
  onFollowUp?: (text: string) => void;
}) {
  const hasPreview = artifact.kind !== 'code';
  const [tab, setTab] = useState<'preview' | 'source'>('preview');
  const [width, setWidth] = useState(() => {
    try { return clampWidth(Number(localStorage.getItem(WIDTH_KEY)) || 400); }
    catch { return 400; }
  });
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const drag = useRef(false);

  // Escape dismisses the canvas (it is an overlay drawer at narrow widths).
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose(); };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(artifact.code);
      setCopied(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 2000);
    } catch { /* Clipboard unavailable. */ }
  };

  const resize = (next: number) => {
    const clamped = clampWidth(next);
    setWidth(clamped);
    try { localStorage.setItem(WIDTH_KEY, String(clamped)); } catch { /* ignore */ }
  };

  const shownTab = hasPreview ? tab : 'source';
  const previewLabel = artifact.kind === 'openui' ? 'Rendered' : 'Preview';
  // Give bare HTML fragments a legible baseline — the sandboxed document
  // inherits neither the app's fonts nor its theme, so inject one.
  const previewDoc = useMemo(() => {
    if (artifact.kind !== 'html') return artifact.code;
    const dark = document.documentElement.dataset.theme === 'dark'
      || (document.documentElement.dataset.theme !== 'light' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    const base = `<style>html{color-scheme:${dark ? 'dark' : 'light'}}body{font-family:'IBM Plex Sans',system-ui,sans-serif;margin:0;padding:16px;line-height:1.55;font-size:14px;background:${dark ? '#1f2521' : '#ffffff'};color:${dark ? '#e5eae4' : '#26382f'}}</style>`;
    return /<head[^>]*>/i.test(artifact.code)
      ? artifact.code.replace(/<head[^>]*>/i, match => match + base)
      : base + artifact.code;
  }, [artifact]);

  return <aside className="canvas-panel" style={{ '--canvas-w': `${width}px` } as React.CSSProperties} aria-label={`Canvas: ${artifact.title}`}>
    <div
      className="canvas-resize"
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize canvas"
      tabIndex={0}
      onKeyDown={event => {
        if (event.key === 'ArrowLeft') { event.preventDefault(); resize(width + 24); }
        if (event.key === 'ArrowRight') { event.preventDefault(); resize(width - 24); }
      }}
      onPointerDown={event => {
        drag.current = true;
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={event => { if (drag.current) resize(window.innerWidth - event.clientX); }}
      onPointerUp={event => { drag.current = false; event.currentTarget.releasePointerCapture(event.pointerId); }}
      onPointerCancel={() => { drag.current = false; }}
    />
    <header className="canvas-header">
      <div className="canvas-title">
        <strong>{artifact.title}</strong>
        <small>{artifact.kind === 'openui' ? 'Generative UI' : artifact.language || 'code'}</small>
      </div>
      <button className="icon-button" aria-label="Close canvas" title="Close canvas" onClick={onClose}><X size={16} /></button>
    </header>
    <div className="canvas-toolbar">
      {hasPreview && <div role="group" aria-label="Canvas view" className="canvas-tabs">
        <button className="secondary" aria-pressed={shownTab === 'preview'} onClick={() => setTab('preview')}>{previewLabel}</button>
        <button className="secondary" aria-pressed={shownTab === 'source'} onClick={() => setTab('source')}>Source</button>
      </div>}
      {versionCount > 1 && <div className="canvas-versions" aria-label={`Version ${artifact.version} of ${versionCount}`}>
        <button className="icon-button" aria-label="Previous version" disabled={artifact.version <= 1} onClick={() => onVersion(artifact.version - 1)}><ChevronLeft size={15} /></button>
        <span aria-hidden="true">{artifact.version} / {versionCount}</span>
        <button className="icon-button" aria-label="Next version" disabled={artifact.version >= versionCount} onClick={() => onVersion(artifact.version + 1)}><ChevronRight size={15} /></button>
      </div>}
      <button className="secondary canvas-copy" onClick={() => void copy()} aria-label="Copy source" title="Copy source">
        {copied ? <Check size={13} className="text-success" /> : <Copy size={13} />}{copied ? 'Copied!' : 'Copy'}
      </button>
    </div>
    <div className="canvas-body">
      {shownTab === 'preview' && artifact.kind === 'openui' && (
        <Suspense fallback={<pre className="canvas-source"><code>{artifact.code}</code></pre>}>
          <div className="canvas-genui"><GenUiSurface code={artifact.code} onFollowUp={onFollowUp} /></div>
        </Suspense>
      )}
      {shownTab === 'preview' && artifact.kind === 'html' && (
        <>
          <iframe className="canvas-frame" sandbox="" title={artifact.title} srcDoc={previewDoc} />
          <p className="canvas-note">Scripts are disabled in the canvas preview.</p>
        </>
      )}
      {shownTab === 'source' && <pre className="canvas-source" tabIndex={0}><code>{artifact.code}</code></pre>}
    </div>
  </aside>;
}
