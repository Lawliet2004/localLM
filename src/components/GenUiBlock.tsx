// Renders an ```openui fenced block (OpenUI Lang program) as live UI.
// Streams progressively while the model emits the program; falls back to the
// a diagnostic when validation fails; source stays behind an explicit toggle.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Check, Code, Copy, LayoutTemplate, PanelRight } from 'lucide-react';
import { type ActionEvent, type OpenUIError } from '@openuidev/react-lang';
import { ValidatedRenderer } from '../genui/ValidatedRenderer';
import { presentationAction } from '../genui/actions';
import { nativeAvailable } from '../lib/api';
import { openUrl } from '@tauri-apps/plugin-opener';

interface Props {
  code: string;
  streaming?: boolean;
  onFollowUp?: (text: string) => void;
  onOpenCanvas?: () => void;
}

export function GenUiBlock({ code, streaming = false, onFollowUp, onOpenCanvas }: Props) {
  const [showSource, setShowSource] = useState(false);
  const [copied, setCopied] = useState(false);
  const [problems, setProblems] = useState<OpenUIError[]>([]);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);

  const onError = useCallback((errors: OpenUIError[]) => {
    setProblems(errors);
  }, []);

  const onAction = useCallback((event: ActionEvent) => {
    if (streaming) return;
    const action = presentationAction(event);
    if (!action) return;
    if (action.type === 'open_url') {
      const { url } = action;
      if (nativeAvailable) {
        void openUrl(url).catch(() => setProblems([{ code: 'runtime-error', source: 'runtime', message: 'Could not open the link.' }]));
      } else {
        window.open(url, '_blank', 'noopener,noreferrer');
      }
      return;
    }
    onFollowUp?.(action.message);
  }, [onFollowUp, streaming]);

  const firstError = useMemo(() => problems.find(error => error.severity !== 'warning'), [problems]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 2000);
    } catch {
      /* Clipboard unavailable. */
    }
  };

  return (
    <div className="genui-block" data-testid="genui-block">
      <div className="genui-block-header">
        <span>Generative UI</span>
        <span className="genui-block-actions">
            <button
              type="button"
              className="code-block-copy-btn"
              onClick={() => setShowSource(value => !value)}
              aria-pressed={showSource}
              title={showSource ? 'Show rendered UI' : 'Show OpenUI Lang source'}
            >
              {showSource ? <LayoutTemplate size={12} /> : <Code size={12} />}
              <span>{showSource ? 'Rendered' : 'Source'}</span>
            </button>
          {!streaming && onOpenCanvas && (
            <button
              type="button"
              className="code-block-copy-btn"
              onClick={onOpenCanvas}
              aria-label="Open in canvas"
              title="Open in canvas"
            >
              <PanelRight size={12} />
              <span>Canvas</span>
            </button>
          )}
          <button
            type="button"
            className="code-block-copy-btn"
            onClick={() => void copy()}
            aria-label="Copy UI source"
            title="Copy UI source"
          >
            {copied ? <Check size={12} className="text-success" /> : <Copy size={12} />}
            <span>{copied ? 'Copied!' : 'Copy'}</span>
          </button>
        </span>
      </div>
      {showSource ? (
        <pre className="code-block-pre"><code>{code}</code></pre>
      ) : (
        <div className="genui-block-body">
          <ValidatedRenderer
            response={code}
            isStreaming={streaming}
            onAction={onAction}
            onError={onError}
          />
        </div>
      )}
      {!streaming && firstError && (
        <p className="genui-block-note" role="status" title={firstError.message}>
          Rendered with issues: {firstError.code}
        </p>
      )}
    </div>
  );
}
