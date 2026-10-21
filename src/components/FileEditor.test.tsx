import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { FileEditor } from './FileEditor';
import type { EditorHandle } from './FileEditor';

const mocks = vi.hoisted(() => ({ editorRead: vi.fn(), editorSave: vi.fn(), openPath: vi.fn() }));
vi.mock('../lib/api', () => ({ api: mocks, errorMessage: (e: unknown) => e instanceof Error ? e.message : String(e) }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openPath: mocks.openPath }));
// Exercise document lifecycle without depending on jsdom's range geometry.
// The real CodeMirror engine is exercised by the browser acceptance suite.
vi.mock('./CodeEditor', () => ({ CodeEditor: ({ initialContent, path, onChange, onSave }: { initialContent: string; path: string; onChange: (value: string) => void; onSave: () => void }) =>
  <textarea aria-label={`Edit ${path}`} defaultValue={initialContent} onChange={event => onChange(event.target.value)} onKeyDown={event => { if (event.ctrlKey && event.key === 's') { event.preventDefault(); onSave(); } }} />,
}));

describe('FileEditor', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.editorRead.mockResolvedValue({ content: 'old\n', sha256: 'hash-old' });
    mocks.editorSave.mockResolvedValue({ content: 'new\n', sha256: 'hash-new' });
  });
  const props = () => ({ root: '/project', path: 'main.ts', active: true, onDirty: vi.fn(), onSaved: vi.fn(), register: vi.fn(), requestDialog: vi.fn().mockResolvedValue({ action: 'cancel' }) });

  it('saves the complete document with the current hash and preserves CRLF and BOM', async () => {
    const user = userEvent.setup();
    mocks.editorRead.mockResolvedValue({ content: '\ufeffold\r\n', sha256: 'hash-old' });
    const callbacks = props();
    render(<FileEditor {...callbacks} />);
    fireEvent.change(await screen.findByLabelText('Edit main.ts'), { target: { value: '\ufeffnew\n' } });
    expect(callbacks.onDirty).toHaveBeenLastCalledWith(true);
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(mocks.editorSave).toHaveBeenCalledWith('/project', 'main.ts', '\ufeffnew\r\n', 'hash-old');
    await waitFor(() => expect(callbacks.onDirty).toHaveBeenLastCalledWith(false));
    expect(callbacks.onSaved).toHaveBeenCalledTimes(1);
  });

  it('keeps edits open on conflict, refuses to discard on Cancel, and reloads only after confirmation', async () => {
    const user = userEvent.setup();
    const callbacks = props();
    mocks.editorSave.mockRejectedValue(new Error('This file changed on disk.'));
    render(<FileEditor {...callbacks} />);
    fireEvent.change(await screen.findByLabelText('Edit main.ts'), { target: { value: 'my draft' } });
    await user.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('This file changed on disk.');
    expect(screen.getByLabelText('Edit main.ts')).toHaveValue('my draft');
    await user.click(screen.getByRole('button', { name: 'Reload file' }));
    expect(mocks.editorRead).toHaveBeenCalledTimes(1);
    callbacks.requestDialog.mockResolvedValue({ action: 'confirm' });
    mocks.editorRead.mockResolvedValue({ content: 'external', sha256: 'hash-external' });
    await user.click(screen.getByRole('button', { name: 'Reload file' }));
    await waitFor(() => expect(screen.getByLabelText('Edit main.ts')).toHaveValue('external'));
  });

  it('deduplicates concurrent saves and leaves new edits dirty while a save completes', async () => {
    let finish!: (value: unknown) => void;
    mocks.editorSave.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
    let handle: EditorHandle | null = null;
    const callbacks = props();
    render(<FileEditor {...callbacks} register={value => { handle = value; }} />);
    fireEvent.change(await screen.findByLabelText('Edit main.ts'), { target: { value: 'first edit' } });
    let pending!: Promise<boolean>;
    act(() => { pending = handle!.save(); void handle!.save(); });
    expect(mocks.editorSave).toHaveBeenCalledTimes(1);
    fireEvent.change(screen.getByLabelText('Edit main.ts'), { target: { value: 'later edit' } });
    await act(async () => { finish({ content: 'first edit', sha256: 'hash-first' }); expect(await pending).toBe(false); });
    expect(handle!.dirty()).toBe(true);
    expect(screen.getByLabelText('Edit main.ts')).toHaveValue('later edit');
    mocks.editorSave.mockResolvedValue({ content: 'later edit', sha256: 'hash-later' });
    await act(async () => { await handle!.save(); });
    expect(mocks.editorSave).toHaveBeenLastCalledWith('/project', 'main.ts', 'later edit', 'hash-first');
  });

  it('drops reads from a previous workspace and reports failures with a retry action', async () => {
    let finish!: (value: unknown) => void;
    mocks.editorRead.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    mocks.editorRead.mockResolvedValue({ content: 'project B', sha256: 'b' });
    const callbacks = props();
    const { rerender } = render(<FileEditor {...callbacks} />);
    rerender(<FileEditor {...callbacks} root="/other-project" />);
    expect(await screen.findByLabelText('Edit main.ts')).toHaveValue('project B');
    await act(async () => { finish({ content: 'project A', sha256: 'a' }); });
    expect(screen.getByLabelText('Edit main.ts')).toHaveValue('project B');
    mocks.editorRead.mockRejectedValue(new Error('Not UTF-8 text'));
    rerender(<FileEditor {...callbacks} path="binary" />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Not UTF-8 text');
    expect(screen.getByRole('button', { name: 'Reload file' })).toBeEnabled();
  });
});
