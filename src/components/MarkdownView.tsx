import { Children, isValidElement, lazy, memo, Suspense, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { PanelRight, X } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import type { PluggableList } from 'unified';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import 'katex/dist/katex.min.css';
import { citationPlugin } from '../lib/sources';
import { mathMarkdown } from '../lib/mathMarkdown';
import { CodeBlock } from './CodeBlock';

// The OpenUI runtime (~300 kB) only downloads when a message actually
// contains an ```openui block; until then the plain code block shows.
const GenUiBlock = lazy(() => import('./GenUiBlock').then(module => ({ default: module.GenUiBlock })));

function ImageLightbox({ src, alt }: { src?: string; alt?: string }) {
  const [isOpen, setIsOpen] = useState(false);
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    if (isOpen) dialog.current?.showModal?.();
    else dialog.current?.close?.();
  }, [isOpen]);
  if (!src) return <span className="image-reference">Image: {alt || 'image'}</span>;
  return (
    <>
      <button type="button" className="image-preview-wrapper" onClick={() => setIsOpen(true)} aria-label={`Enlarge ${alt || 'image'}`} title="Enlarge image">
        <img src={src} alt={alt || 'Image preview'} className="image-preview-thumb" />
      </button>
      <dialog ref={dialog} className="image-lightbox-overlay" aria-label={alt || 'Image preview'} onCancel={() => setIsOpen(false)} onClose={() => setIsOpen(false)} onClick={e => { if (e.target === e.currentTarget) setIsOpen(false); }}>
          <div className="image-lightbox-modal" onClick={e => e.stopPropagation()}>
            <img src={src} alt={alt || 'Enlarged image'} className="lightbox-img" />
            <button
              type="button"
              className="lightbox-close-btn"
              onClick={() => setIsOpen(false)}
              aria-label="Close preview"
            >
              <X size={18} />
            </button>
          </div>
      </dialog>
    </>
  );
}

export interface MarkdownViewProps {
  content: string;
  /** Assistant messages may render interactive blocks (OpenUI, Canvas). */
  assistant?: boolean;
  /** True while content may still be growing. */
  streaming?: boolean;
  /** Serialized citation map: citation id -> URL. */
  sourceLinks?: string;
  onSendPrompt?: (text: string) => void;
  onOpenCanvas?: (code: string, language: string) => void;
  /** Visual answers only permit links already grounded in their evidence. */
  allowedLinkUrls?: readonly string[];
  allowImages?: boolean;
}

/**
 * The one markdown renderer for assistant prose, user messages and exposed
 * reasoning. Unclosed fences from a still-streaming block already parse as
 * code, so incomplete markdown stays readable without rewriting the source.
 */
export const MarkdownView = memo(function MarkdownView({ content, assistant = false, streaming = false, sourceLinks = '{}', onSendPrompt, onOpenCanvas, allowedLinkUrls, allowImages = true }: MarkdownViewProps) {
  const renderedContent = useMemo(() => mathMarkdown(content), [content]);
  const plugins = useMemo<PluggableList>(() => {
    let links: Record<string, string> = {};
    try {
      const parsed: unknown = JSON.parse(sourceLinks);
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        links = Object.fromEntries(Object.entries(parsed).filter((entry): entry is [string, string] => typeof entry[1] === 'string'));
      }
    } catch { /* A corrupt citation map must not hide the answer. */ }
    return [remarkGfm, [remarkMath, { singleDollarTextMath: false }], citationPlugin(links)];
  }, [sourceLinks]);
  return (
    <div className={`markdown${streaming && content ? ' is-streaming' : ''}`}>
      <ReactMarkdown
        remarkPlugins={plugins}
        rehypePlugins={[[rehypeKatex, { trust: false, strict: 'ignore', throwOnError: false, maxSize: 20, maxExpand: 1000 }]]}
        components={{
          a: ({ children, href }) => !href || (allowedLinkUrls && !allowedLinkUrls.includes(href)) ? <span>{children}</span> : <a href={href} target="_blank" rel="noopener noreferrer">{children}</a>,
          img: ({ src, alt }) => allowImages ? <ImageLightbox src={src} alt={alt} /> : <span>{alt || 'Image reference'}</span>,
          table: ({ children }) => <div className="markdown-table-scroll" role="region" aria-label="Table" tabIndex={0}><table>{children}</table></div>,
          pre: ({ children }) => {
            const child = Children.toArray(children)[0];
            if (isValidElement<{ className?: string; children?: ReactNode }>(child)) {
              const language = /language-(\S+)/.exec(child.props.className || '')?.[1] ?? '';
              const code = String(child.props.children ?? '').replace(/\n$/, '');
              if (assistant && (language === 'openui' || language === 'openui-lang')) {
                return (
                  <Suspense fallback={<div className="genui-block"><div className="genui-block-header"><span>Generative UI</span></div><p className="genui-block-note">Rendering UI…</p></div>}>
                    <GenUiBlock code={code} streaming={streaming} onFollowUp={onSendPrompt} onOpenCanvas={onOpenCanvas ? () => onOpenCanvas(code, language) : undefined} />
                  </Suspense>
                );
              }
              return (
                <CodeBlock
                  language={language}
                  code={code}
                  streaming={streaming}
                  actions={assistant && onOpenCanvas ? (
                    <button
                      type="button"
                      className="code-block-copy-btn"
                      onClick={() => onOpenCanvas(code, language)}
                      aria-label="Open in canvas"
                      title="Open in canvas"
                    >
                      <PanelRight size={12} />
                      <span>Canvas</span>
                    </button>
                  ) : undefined}
                />
              );
            }
            return <pre>{children}</pre>;
          },
          code: ({ className, children, ...rest }) => <code className={className} {...rest}>{children}</code>,
        }}
      >
        {renderedContent}
      </ReactMarkdown>
    </div>
  );
});
