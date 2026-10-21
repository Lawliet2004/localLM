import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { CodeBlock } from './CodeBlock';

it('renders a labelled frame with highlighted grammar spans', () => {
  render(<CodeBlock code="const x = 1;" language="ts" />);
  const wrapper = screen.getByLabelText('typescript code block');
  expect(wrapper.querySelector('.code-block-lang')).toHaveTextContent('typescript');
  expect(wrapper.querySelector('.hljs-keyword')).toHaveTextContent('const');
  expect(wrapper.querySelector('.hljs-number')).toHaveTextContent('1');
  expect(wrapper.querySelector('code')).toHaveClass('language-typescript');
});

it('renders unreadable languages as escaped plain text without a language class', () => {
  const markup = '<img src=x onerror=alert(1)>';
  const { container } = render(<CodeBlock code={markup} language="cobol" />);
  const code = container.querySelector('code')!;
  expect(code).not.toHaveClass('language-cobol');
  expect(code).toHaveTextContent(markup);
  expect(code.querySelector('img')).toBeNull();
  expect(screen.getByLabelText('cobol code block').querySelector('.code-block-lang')).toHaveTextContent('cobol');
});

it('copies the exact underlying source, not the rendered markup', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.assign(navigator, { clipboard: { writeText } });
  const source = 'const html = "<b>x</b>";';
  render(<CodeBlock code={source} language="typescript" />);
  fireEvent.click(screen.getByRole('button', { name: 'Copy code block' }));
  expect(writeText).toHaveBeenCalledWith(source);
  expect(await screen.findByText('Copied!')).toBeInTheDocument();
});

it('offers a wrap toggle only when a line exceeds the viewport width', () => {
  const long = `const url = "${'x'.repeat(140)}";`;
  const { container, rerender } = render(<CodeBlock code="short" language="js" />);
  expect(screen.queryByRole('button', { name: /wrap/i })).toBeNull();
  rerender(<CodeBlock code={long} language="js" />);
  const wrapBtn = screen.getByRole('button', { name: 'Wrap long lines' });
  expect(wrapBtn).toHaveAttribute('aria-pressed', 'false');
  fireEvent.click(wrapBtn);
  expect(container.querySelector('.code-block-wrapper')).toHaveClass('is-wrap');
  expect(screen.getByRole('button', { name: 'Disable line wrap' })).toHaveAttribute('aria-pressed', 'true');
});

it('defers highlighting for oversized blocks while still streaming', () => {
  const big = `const data = "${'x'.repeat(21_000)}";`;
  const { container } = render(<CodeBlock code={big} language="ts" streaming />);
  const code = container.querySelector('code')!;
  expect(code).toHaveTextContent('const data =');
  expect(code.querySelector('.hljs-keyword')).toBeNull();
});

it('keeps extra toolbar actions alongside copy', () => {
  render(<CodeBlock code="x" language="js" actions={<button type="button">Canvas</button>} />);
  expect(screen.getByRole('button', { name: 'Canvas' })).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Copy code block' })).toBeInTheDocument();
});

it('reports clipboard failure while keeping the source keyboard accessible', async () => {
  Object.assign(navigator, { clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) } });
  render(<CodeBlock code="const value = 1;" language="ts" />);
  fireEvent.click(screen.getByRole('button', { name: 'Copy code block' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Could not copy');
  expect(screen.getByLabelText('typescript source')).toHaveAttribute('tabindex', '0');
});
