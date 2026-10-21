import { useEffect, useLayoutEffect, useState } from 'react';
import { APPEARANCE_STORAGE_KEY, applyAppearance, defaultAppearance, loadAppearance, normalizeAppearance, resolveTheme, systemPrefersDark, type AppearancePreferences } from './appearance';

export function useAppearance() {
  const [preferences, setPreferences] = useState(loadAppearance);
  const [systemDark, setSystemDark] = useState(systemPrefersDark);
  const [saveError, setSaveError] = useState('');
  const theme = resolveTheme(preferences.theme, systemDark);

  useLayoutEffect(() => { applyAppearance(preferences, theme); }, [preferences, theme]);

  useEffect(() => {
    if (typeof window.matchMedia !== 'function') return;
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const update = () => setSystemDark(media.matches);
    update();
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }, []);

  useEffect(() => {
    const sync = (event: StorageEvent) => {
      if (event.key === APPEARANCE_STORAGE_KEY || event.key === 'locallm-theme' || event.key === null) {
        setPreferences(loadAppearance());
        setSaveError('');
      }
    };
    window.addEventListener('storage', sync);
    return () => window.removeEventListener('storage', sync);
  }, []);

  function update(patch: Partial<AppearancePreferences>) {
    const next = normalizeAppearance({ ...preferences, ...patch });
    setPreferences(next);
    try {
      localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(next));
      setSaveError('');
      // Keep compatibility with the previous light/dark preference.
      try { localStorage.setItem('locallm-theme', next.theme); } catch { /* The full preference was saved. */ }
    } catch {
      setSaveError('Your changes are applied for this session, but could not be saved on this device. Allow local storage and try again.');
    }
  }

  return { preferences, theme, saveError, update, reset: () => update({ ...defaultAppearance }) };
}
