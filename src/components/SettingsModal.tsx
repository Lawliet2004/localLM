// Settings open as a large floating modal over the chat surface. A native
// <dialog> supplies the backdrop, Escape handling and focus containment;
// below 760px it becomes a full-screen sheet with a horizontal nav strip.
import { useEffect, useRef, type ReactNode } from 'react';
import type { Page } from './Sidebar';
import { SettingsNavigation } from './SettingsNavigation';

export function SettingsModal({ page, onPage, onClose, children }: {
  page: Page;
  onPage: (page: Page) => void;
  onClose: () => void;
  children: ReactNode;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  // Capture the trigger before the dialog takes focus so close restores it.
  const opener = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (!element.open) {
      if (typeof element.showModal === 'function') element.showModal();
      else element.setAttribute('open', '');
    }
    element.querySelector<HTMLElement>('input, button')?.focus();
    return () => {
      if (element.open && typeof element.close === 'function') element.close();
      if (opener.current?.isConnected) opener.current.focus();
    };
  }, []);

  return <dialog
    ref={dialog}
    className="settings-modal"
    aria-label="Settings"
    onCancel={event => { event.preventDefault(); onClose(); }}
    onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); onClose(); } }}
    onClick={event => { if (event.target === event.currentTarget) onClose(); }}
  >
    <div className="settings-modal-body">
      <SettingsNavigation page={page} onPage={onPage} onClose={onClose} />
      <div className="settings-modal-content">{children}</div>
    </div>
  </dialog>;
}
