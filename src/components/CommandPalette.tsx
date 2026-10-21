import { useEffect, useRef, useState } from 'react';
import { Search } from 'lucide-react';
export interface PaletteCommand { id: string; label: string; keywords?: string; run: () => void; disabled?: boolean }
export function CommandPalette({ open, onClose, commands }: { open: boolean; onClose: () => void; commands: PaletteCommand[] }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [query, setQuery] = useState('');
  const [index, setIndex] = useState(0);
  const matches = commands.filter(command => `${command.label} ${command.keywords ?? ''}`.toLowerCase().includes(query.toLowerCase()));
  useEffect(() => { if (open) { setQuery(''); setIndex(0); dialog.current?.showModal?.(); } else dialog.current?.close?.(); }, [open]);
  useEffect(() => {
    if (open && matches[index]) document.getElementById(`command-${matches[index].id}`)?.scrollIntoView?.({ block: 'nearest' });
  }, [open, index, query, matches.length]);
  function choose(command?: PaletteCommand) { if (command && !command.disabled) { onClose(); command.run(); } }
  return <dialog ref={dialog} className="command-palette" aria-label="Command palette" onCancel={onClose} onClose={onClose} onClick={event => { if (event.target === event.currentTarget) onClose(); }}>
    <div className="command-palette-content" onKeyDown={event => {
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setIndex(current => matches.length ? (current + (event.key === 'ArrowDown' ? 1 : -1) + matches.length) % matches.length : 0); }
      if (event.key === 'Enter') { event.preventDefault(); choose(matches[index]); }
    }}><label><Search size={18} /><input autoFocus role="combobox" aria-label="Search commands" aria-expanded={true} aria-controls="palette-results" aria-activedescendant={matches[index] ? `command-${matches[index].id}` : undefined} value={query} placeholder="Search commands…" onChange={event => { setQuery(event.target.value); setIndex(0); }} /></label>
      <div id="palette-results" role="listbox" aria-label="Commands">{matches.map((command, i) => <div id={`command-${command.id}`} key={command.id} role="option" aria-selected={i === index} aria-disabled={command.disabled} onMouseEnter={() => setIndex(i)}><button tabIndex={-1} disabled={command.disabled} onClick={() => choose(command)}>{command.label}</button></div>)}</div>
      {!matches.length && <p role="status">No matching commands.</p>}
      <small>↑ ↓ Navigate · Enter Select · Esc Close</small>
    </div>
  </dialog>;
}
