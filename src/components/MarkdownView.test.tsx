import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { MarkdownView } from './MarkdownView';

describe('answer rendering', () => {
  it('keeps punctuation in language labels such as C#', () => {
    const { container } = render(<MarkdownView content={'```c#\npublic class App {}\n```'} />);
    expect(container.querySelector('code.language-csharp')).not.toBeNull();
  });
  it('renders prose, code, tasks and a keyboard-accessible comparison table together', () => {
    const { container } = render(<MarkdownView assistant content={'## Fix\n\nUse **a guard** before `items.map`.\n\n```cpp\nint count = 1;\n```\n\n| Before | After |\n| --- | --- |\n| Crash | Safe |\n\n- [x] Guard added\n- [ ] Run tests'} />);
    expect(screen.getByRole('heading', { name: 'Fix' })).toBeInTheDocument();
    expect(container.querySelector('.hljs-type')).toHaveTextContent('int');
    expect(screen.getByRole('region', { name: 'Table' })).toHaveAttribute('tabindex', '0');
    expect(screen.getByRole('cell', { name: 'Safe' })).toBeInTheDocument();
    expect(screen.getAllByRole('checkbox')).toHaveLength(2);
    expect(screen.getAllByRole('checkbox')[0]).toBeChecked();
  });

  it('typesets inline and block math without treating currency or source code as equations', () => {
    const { container } = render(<MarkdownView content={'Inline $$x^2$$. Costs $5 and $10.\n\n$$\n\\frac{a}{b}\n$$\n\n```python\nprice = "$5"\n```'} />);
    expect(container.querySelectorAll('.katex')).toHaveLength(2);
    expect(container.querySelector('.katex-display')).toBeInTheDocument();
    expect(screen.getByText(/Costs \$5 and \$10/)).toBeInTheDocument();
    expect(container.querySelector('code.language-python')).toHaveTextContent('price = "$5"');
    expect(container.querySelector('math')).not.toBeNull();
  });

  it('recovers from incomplete math and an unfinished code fence during streaming', () => {
    const { container, rerender } = render(<MarkdownView content={'$$\n\\frac{a'} streaming />);
    expect(container.querySelector('.markdown')).toBeInTheDocument();
    rerender(<MarkdownView content={'```ts\nconst value ='} streaming />);
    expect(container.querySelector('code.language-typescript')).toHaveTextContent('const value =');
  });

  it('renders the LaTeX delimiters models use without changing code samples', () => {
    const { container } = render(<MarkdownView content={'Inline \\(x^2\\).\n\n\\[\\frac{a}{b}\\]\n\n```python\nexpression = "\\(x\\)"\n```'} />);
    expect(container.querySelectorAll('.katex')).toHaveLength(2);
    expect(container.querySelector('code.language-python')).toHaveTextContent('expression = "\\(x\\)"');
  });

  it.each(['{broken', 'null', '[]', '{"S1":7}'])('keeps the answer visible with malformed citation metadata %s', sourceLinks => {
    render(<MarkdownView content="Readable answer [S1]." sourceLinks={sourceLinks} />);
    expect(screen.getByText('Readable answer [S1].')).toBeInTheDocument();
  });

  it('keeps model HTML literal and disables trusted TeX commands', () => {
    const { container } = render(<MarkdownView content={'<script>alert(1)</script>\n\n```html\n<img src=x onerror=alert(1)>\n```\n\n$$\\href{javascript:alert(1)}{click}$$'} />);
    expect(container.querySelector('script')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('a[href^="javascript:"]')).toBeNull();
    expect(container.querySelector('code')).toHaveTextContent('<img src=x onerror=alert(1)>');
  });

  it('preserves grounded-link and image restrictions for visual prose', () => {
    render(<MarkdownView content={'[Verified](https://example.com/docs) [Other](https://other.example)\n\n![Photo](https://other.example/image.png)'} allowedLinkUrls={['https://example.com/docs']} allowImages={false} />);
    expect(screen.getByRole('link', { name: 'Verified' })).toHaveAttribute('href', 'https://example.com/docs');
    expect(screen.queryByRole('link', { name: 'Other' })).toBeNull();
    expect(screen.queryByRole('img')).toBeNull();
    expect(screen.getByText('Photo')).toBeInTheDocument();
  });

  it('opens and closes an answer image preview', () => {
    render(<MarkdownView content={'![Diagram](https://example.com/diagram.png)'} />);
    fireEvent.click(screen.getByRole('button', { name: 'Enlarge Diagram' }));
    expect(screen.getAllByAltText('Diagram')).toHaveLength(2);
    fireEvent.click(screen.getByRole('button', { name: 'Close preview', hidden: true }));
  });
});

it('preserves a code block node and wrap preference when later prose streams', () => {
  const content = `\`\`\`ts\nconst longLine = "${'a'.repeat(150)}";\n\`\`\`\n\nLater`;
  const view = render(<MarkdownView content={content} streaming />);
  const source = screen.getByLabelText('typescript source');
  fireEvent.click(screen.getByRole('button', { name: 'Wrap long lines' }));
  view.rerender(<MarkdownView content={`${content} text`} streaming />);
  expect(screen.getByLabelText('typescript source')).toBe(source);
  expect(screen.getByRole('button', { name: 'Disable line wrap' })).toHaveAttribute('aria-pressed', 'true');
  view.rerender(<MarkdownView content={`${content} text`} streaming={false} />);
  expect(screen.getByLabelText('typescript source')).toBe(source);
  expect(screen.getByRole('button', { name: 'Disable line wrap' })).toHaveAttribute('aria-pressed', 'true');
});
