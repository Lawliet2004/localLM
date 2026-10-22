import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { DiffLines, diffCounts } from './DiffView';
it('shows source coordinates and syntax on changed sections', () => {
  const { container } = render(<DiffLines path="src/app.ts" diff={'--- a/src/app.ts\n+++ b/src/app.ts\n@@ -10,2 +20,2 @@\n const unchanged = true;\n-const old = 1;\n+const next = 2;'} />);
  const added = container.querySelector('.diff-line.add')!;
  expect([...added.querySelectorAll('.diff-gutter')].map(gutter => gutter.textContent)).toEqual(['', '21']);
  expect(added.querySelector('.hljs-keyword')).toHaveTextContent('const');
  expect(screen.getByLabelText('File changes')).toHaveAttribute('tabindex', '0');
});
it('counts source beginning with ++/-- without counting file headers', () => {
  const diff = '--- a/a\n+++ b/a\n@@ -1 +1 @@\n--- value\n+++ next\ndiff --git a/b b/b\n--- a/b\n+++ b/b\n@@ -0,0 +1 @@\n+new';
  expect(diffCounts(diff)).toEqual({ added: 2, removed: 1 });
});

it('collapses unchanged context while keeping changed lines and real coordinates visible', async () => {
  const { fireEvent } = await import('@testing-library/react');
  const context = Array.from({ length: 20 }, (_, index) => ` context ${index + 1}`).join('\n');
  const { container } = render(<DiffLines diff={`@@ -1,21 +1,21 @@\n${context}\n-old\n+new`} />);
  expect(screen.getByRole('button', { name: 'Show 14 unchanged lines' })).toBeInTheDocument();
  expect(screen.queryByText('context 10')).toBeNull();
  expect(container.querySelector('.diff-line.add .diff-content')).toHaveTextContent('+new');
  const toggle = screen.getByRole('button', { name: 'Show 14 unchanged lines' });
  toggle.focus(); fireEvent.click(toggle);
  expect(screen.getByRole('button', { name: 'Collapse unchanged lines' })).toBe(toggle);
  expect(toggle).toHaveFocus();
  expect(screen.getByText('context 10')).toBeInTheDocument();
  fireEvent.click(toggle);
  expect(toggle).toHaveFocus();
  expect(toggle).toHaveAttribute('aria-expanded', 'false');
  expect(screen.queryByText('context 10')).toBeNull();
});
