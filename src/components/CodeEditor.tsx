import { useEffect, useRef } from 'react';
import { basicSetup } from 'codemirror';
import { EditorState } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { indentWithTab } from '@codemirror/commands';
import { HighlightStyle, syntaxHighlighting } from '@codemirror/language';
import { tags } from '@lezer/highlight';
import { javascript } from '@codemirror/lang-javascript';
import { json } from '@codemirror/lang-json';
import { python } from '@codemirror/lang-python';
import { rust } from '@codemirror/lang-rust';
import { html } from '@codemirror/lang-html';
import { css } from '@codemirror/lang-css';
import { markdown } from '@codemirror/lang-markdown';
import { cpp } from '@codemirror/lang-cpp';

function language(path: string) {
  const ext = path.split('.').pop()?.toLowerCase();
  if (['js', 'jsx', 'mjs', 'cjs', 'ts', 'tsx'].includes(ext ?? '')) return javascript({ typescript: ext === 'ts' || ext === 'tsx', jsx: ext === 'jsx' || ext === 'tsx' });
  if (ext === 'json') return json();
  if (ext === 'py') return python();
  if (ext === 'rs') return rust();
  if (ext === 'html' || ext === 'htm') return html();
  if (ext === 'css') return css();
  if (ext === 'md' || ext === 'mdx') return markdown();
  if (['c', 'h', 'cpp', 'hpp', 'cc'].includes(ext ?? '')) return cpp();
  return [];
}

const highlighting = HighlightStyle.define([
  { tag: [tags.keyword, tags.operator], color: 'var(--editor-keyword, #c792ea)' },
  { tag: [tags.string, tags.regexp], color: 'var(--accent)' },
  { tag: [tags.number, tags.bool, tags.null], color: 'var(--warn)' },
  { tag: tags.comment, color: 'var(--muted)', fontStyle: 'italic' },
  { tag: [tags.function(tags.variableName), tags.typeName, tags.tagName], color: 'var(--folder)' },
]);

export function CodeEditor({ initialContent, path, onChange, onSave, onCursor }: {
  initialContent: string; path: string; onChange: (content: string) => void;
  onSave: () => void; onCursor: (line: number, column: number) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const callbacks = useRef({ onChange, onSave, onCursor });
  callbacks.current = { onChange, onSave, onCursor };
  useEffect(() => {
    if (!host.current) return;
    const view = new EditorView({
      parent: host.current,
      state: EditorState.create({ doc: initialContent, extensions: [
        basicSetup, language(path), syntaxHighlighting(highlighting),
        EditorView.contentAttributes.of({ 'aria-label': `Edit ${path}`, spellcheck: 'false' }),
        keymap.of([{ key: 'Mod-s', run: () => { callbacks.current.onSave(); return true; } }, indentWithTab]),
        EditorView.updateListener.of(update => {
          if (update.docChanged) callbacks.current.onChange(update.state.doc.toString());
          if (update.selectionSet || update.docChanged) {
            const pos = update.state.selection.main.head;
            const line = update.state.doc.lineAt(pos);
            callbacks.current.onCursor(line.number, pos - line.from + 1);
          }
        }),
        EditorView.theme({
          '&': { height: '100%', color: 'var(--text)', backgroundColor: 'var(--background)', fontSize: 'var(--code-font-size, 13px)' },
          '.cm-scroller': { overflow: 'auto', fontFamily: 'var(--font-mono, Consolas, monospace)' },
          '.cm-content': { caretColor: 'var(--text)', padding: '12px 0' },
          '.cm-line': { padding: '0 16px' },
          '.cm-gutters': { backgroundColor: 'var(--sidebar)', color: 'var(--muted)', borderColor: 'var(--border)' },
          '.cm-activeLine, .cm-activeLineGutter': { backgroundColor: 'var(--hover)' },
          '.cm-cursor': { borderLeftColor: 'var(--text)' },
          '&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection': { backgroundColor: 'var(--editor-selection, #596c994a)' },
          '.cm-panels, .cm-tooltip': { backgroundColor: 'var(--elevated)', color: 'var(--text)', borderColor: 'var(--border)' },
          '.cm-searchMatch': { backgroundColor: '#f1b45440' },
        }),
      ] }),
    });
    return () => view.destroy();
    // The caller remounts only on explicit reload. Edits never recreate a view.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return <div className="code-editor" ref={host} />;
}
