import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Check, Copy, WrapText } from 'lucide-react';
import { highlightCode, languageLabel } from '../lib/highlight';

export interface CodeBlockProps {
  code: string;
  language?: string | null;
  /** Extra toolbar actions such as the Canvas button. */
  actions?: ReactNode;
  /** True while the block may still be growing. Large streaming blocks defer
      highlighting to the completed render to keep token-rate updates cheap. */
  streaming?: boolean;
  defaultWrap?: boolean;
  className?: string;
  /** Accessible label describing this block (default: "<lang> code block"). */
  ariaLabel?: string;
  lineNumbers?: boolean;
}

/** While a block is still streaming, highlighting past this size is deferred. */
const STREAMING_HIGHLIGHT_LIMIT = 20_000;

/**
 * One code frame for every surface: assistant fences, command input, code
 * execution, file reads, structured results and artifacts. Highlighted markup
 * comes from highlight.js, which escapes its input; plain fallbacks are
 * escaped in highlightCode, so innerHTML is safe for untrusted source.
 * Copy always returns the exact underlying source, not the rendered markup.
 */
export const CodeBlock = memo(function CodeBlock({ code, language, actions, streaming = false, defaultWrap = false, className, ariaLabel, lineNumbers = false }: CodeBlockProps) {
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  const [wrap, setWrap] = useState(defaultWrap);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => () => clearTimeout(timer.current), []);

  const highlighted = useMemo(
    () => highlightCode(code, streaming && code.length > STREAMING_HIGHLIGHT_LIMIT ? null : language),
    [code, language, streaming],
  );

  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopyError(false);
      setCopied(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 2000);
    } catch {
      setCopyError(true);
    }
  };

  const label = languageLabel(language);
  const lines = useMemo(() => code.split('\n'), [code]);
  const hasLongLine = lines.some(line => line.length > 120);

  return (
    <div className={`code-block-wrapper${wrap ? ' is-wrap' : ''}${className ? ` ${className}` : ''}`} aria-label={ariaLabel ?? `${label} code block`}>
      <div className="code-block-header">
        <span className="code-block-lang" data-language={highlighted.language ?? 'plain'}>{label}</span>
        <span className="code-block-actions">
          {highlighted.truncated && <span className="code-block-note" title="Highlighting skipped for a large block">plain</span>}
          {actions}
          {hasLongLine && (
            <button
              type="button"
              className="code-block-copy-btn"
              onClick={() => setWrap(value => !value)}
              aria-pressed={wrap}
              aria-label={wrap ? 'Disable line wrap' : 'Wrap long lines'}
              title={wrap ? 'Disable line wrap' : 'Wrap long lines'}
            >
              <WrapText size={12} />
            </button>
          )}
          <button
            type="button"
            className="code-block-copy-btn"
            onClick={() => void onCopy()}
            aria-label="Copy code block"
            title="Copy code"
          >
            {copied ? <Check size={12} className="text-success" /> : <Copy size={12} />}
            <span>{copied ? 'Copied!' : 'Copy'}</span>
          </button>
        </span>
      </div>
      {copyError && <p className="code-block-copy-error" role="alert">Could not copy. Select the code and copy it manually.</p>}
      <div className={lineNumbers ? "code-block-numbered" : undefined}>
      {lineNumbers && lines.length <= 2000 && <pre className="code-block-line-numbers" aria-hidden="true">{lines.map((_,i)=>i+1).join('\n')}</pre>}
      <pre className="code-block-pre" tabIndex={0} aria-label={`${label} source`}>
        <code className={highlighted.language ? `hljs language-${highlighted.language}` : 'hljs'} dangerouslySetInnerHTML={{ __html: highlighted.html }} />
      </pre>
      </div>
    </div>
  );
});
