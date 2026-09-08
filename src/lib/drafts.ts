const drafts = new Map<string, string>();
const keyFor = (id: string | null) => `locallm-draft:${id ?? 'new'}`;

export function loadDraft(id: string | null): string {
  const key = keyFor(id);
  if (drafts.has(key)) return drafts.get(key)!;
  try { return localStorage.getItem(key) ?? ''; }
  catch { return ''; }
}

export function saveDraft(id: string | null, content: string): void {
  const key = keyFor(id);
  drafts.set(key, content);
  try {
    if (content) localStorage.setItem(key, content);
    else localStorage.removeItem(key);
  } catch { throw new Error('Your draft is kept for this session, but could not be saved on this device. Copy it before closing the app.'); }
}

export function moveNewDraft(id: string): void {
  const content = loadDraft(null);
  saveDraft(id, content);
  saveDraft(null, '');
}
