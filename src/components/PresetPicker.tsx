import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { Preset } from '../lib/types';

interface Props {
  conversationId: string | null;
  disabled: boolean;
}

// ponytail: preset is per-conversation server state; the picker reads + writes it, nothing else.
export function PresetPicker({ conversationId, disabled }: Props) {
  const [presets, setPresets] = useState<Preset[]>([]);
  const [active, setActive] = useState('standard');
  const [error, setError] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    api.listPresets().then(setPresets).catch(e => setError(errorMessage(e)));
  }, []);
  useEffect(() => {
    if (!nativeAvailable || !conversationId) { setActive('standard'); return; }
    api.getPreset(conversationId).then(preset => setActive(preset.id)).catch(() => setActive('standard'));
  }, [conversationId]);
  async function change(id: string) {
    if (!nativeAvailable || !conversationId || disabled) return;
    setError('');
    try { await api.setPreset(conversationId, id); setActive(id); }
    catch (e) { setError(errorMessage(e)); }
  }
  const current = presets.find(preset => preset.id === active);
  return (
    <label className="preset-picker">Mode{' '}
      <select aria-label="Runtime mode" value={active} disabled={disabled || !conversationId} onChange={e => void change(e.target.value)}>
        {presets.map(preset => <option key={preset.id} value={preset.id}>{preset.name}</option>)}
      </select>
      {current && <small title={current.description}>{current.description.slice(0, 80)}</small>}
      {error && <small role="alert">{error}</small>}
    </label>
  );
}
