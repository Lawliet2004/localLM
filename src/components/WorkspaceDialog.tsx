import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

export interface DialogOptions {
  title: string;
  message: string;
  input?: string;
  inputLabel?: string;
  confirm: string;
  alternate?: string;
  destructive?: boolean;
}
export interface DialogResult { action: 'confirm' | 'alternate' | 'cancel'; value: string }

/** A keyboard-accessible modal. Cancel is always the safe default. */
export function WorkspaceDialog({ options, onResult }: { options: DialogOptions; onResult: (result: DialogResult) => void }) {
  const [value, setValue] = useState(options.input ?? '');
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const container = ref.current;
    (container?.querySelector('input') ?? container?.querySelector<HTMLButtonElement>('[data-cancel]'))?.focus();
    return () => previous?.focus();
  }, []);
  return createPortal(<div className="workspace-dialog-backdrop" onKeyDown={event => {
    event.stopPropagation();
    if (event.key === 'Escape') { event.preventDefault(); onResult({ action: 'cancel', value }); }
    if (event.key === 'Tab') {
      const focusable = [...(ref.current?.querySelectorAll<HTMLElement>('input, button:not(:disabled)') ?? [])];
      const first = focusable[0], last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
    }
  }}>
    <div ref={ref} className="workspace-dialog" role="dialog" aria-modal="true" aria-labelledby="workspace-dialog-title" aria-describedby="workspace-dialog-message">
      <form onSubmit={event => { event.preventDefault(); if (options.input === undefined || value.trim()) onResult({ action: 'confirm', value: value.trim() }); }}>
        <h2 id="workspace-dialog-title">{options.title}</h2>
        <p id="workspace-dialog-message">{options.message}</p>
        {options.input !== undefined && <label>{options.inputLabel ?? 'Name'}<input value={value} onChange={event => setValue(event.target.value)} autoComplete="off" required maxLength={4096} /></label>}
        <div className="workspace-dialog-actions">
          <button data-cancel type="button" onClick={() => onResult({ action: 'cancel', value })}>Cancel</button>
          {options.alternate && <button type="button" className="danger-text" onClick={() => onResult({ action: 'alternate', value })}>{options.alternate}</button>}
          <button className={options.destructive ? 'danger' : 'primary'} disabled={options.input !== undefined && !value.trim()}>{options.confirm}</button>
        </div>
      </form>
    </div>
  </div>, document.body);
}
