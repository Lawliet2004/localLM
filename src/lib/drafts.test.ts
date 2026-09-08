import { describe, expect, it, vi } from 'vitest';
import { loadDraft, saveDraft, moveNewDraft } from './drafts';

describe('conversation drafts', () => {
  it('keeps chats independent and transfers the new-chat draft on creation', () => {
    saveDraft('draft-a', 'First unsent message');
    saveDraft('draft-b', 'Second unsent message');
    expect(loadDraft('draft-a')).toBe('First unsent message');
    expect(loadDraft('draft-b')).toBe('Second unsent message');
    saveDraft(null, 'Typing ahead');
    moveNewDraft('draft-created');
    expect(loadDraft('draft-created')).toBe('Typing ahead');
    expect(loadDraft(null)).toBe('');
    expect(localStorage.getItem('locallm-draft:draft-created')).toBe('Typing ahead');
    saveDraft('draft-a', '');
    expect(localStorage.getItem('locallm-draft:draft-a')).toBeNull();
    expect(loadDraft('draft-b')).toBe('Second unsent message');
  });
  it('loads a draft from disk and keeps session text if persistence fails', () => {
    localStorage.setItem('locallm-draft:existing', 'Saved across reload');
    expect(loadDraft('existing')).toBe('Saved across reload');
    const failure = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('Quota'); });
    expect(() => saveDraft('quota-draft', 'Keep this text')).toThrow('kept for this session');
    expect(loadDraft('quota-draft')).toBe('Keep this text');
    failure.mockRestore();
  });
});
