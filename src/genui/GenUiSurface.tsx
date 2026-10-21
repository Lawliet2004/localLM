// Bare GenUI render surface for the canvas panel — the same Renderer and
// action wiring GenUiBlock uses, without the message-block chrome.
import { useCallback, useState } from 'react';
import { type ActionEvent } from '@openuidev/react-lang';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ValidatedRenderer } from './ValidatedRenderer';
import { presentationAction } from './actions';
import { nativeAvailable } from '../lib/api';

export function GenUiSurface({ code, onFollowUp }: { code: string; onFollowUp?: (text: string) => void }) {
  const [linkError, setLinkError] = useState(false);
  const onAction = useCallback((event: ActionEvent) => {
    const action = presentationAction(event);
    if (!action) return;
    if (action.type === 'open_url') {
      const { url } = action;
      if (nativeAvailable) {
        void openUrl(url).catch(() => setLinkError(true));
      } else {
        window.open(url, '_blank', 'noopener,noreferrer');
      }
      return;
    }
    onFollowUp?.(action.message);
  }, [onFollowUp]);

  return <div className="genui-surface">
    <ValidatedRenderer response={code} isStreaming={false} onAction={onAction} />
    {linkError && <p role="status">Could not open the link.</p>}
  </div>;
}
