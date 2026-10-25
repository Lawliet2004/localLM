import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { StructuredResultView } from './StructuredResultView';

describe('StructuredResultView', () => {
  it('renders scalar fields as parameter rows and hides envelope bookkeeping', () => {
    render(
      <StructuredResultView
        result={{
          schemaVersion: 'locallm.tool-envelope/1',
          status: 'success',
          isError: false,
          metadata: { family: 'workspace', sideEffects: 'unknown', truncated: false },
          artifacts: [],
          path: 'src/calc.js',
          sha256: 'a'.repeat(64),
          totalLines: 2,
        }}
      />,
    );
    const text = screen.getByText('src/calc.js');
    expect(text.tagName).toBe('CODE');
    expect(screen.getByText('2')).toBeInTheDocument();
    // Envelope bookkeeping never reaches the friendly view.
    const view = document.querySelector('.structured-result');
    expect(view?.textContent).not.toContain('schemaVersion');
    expect(view?.textContent).not.toContain('sideEffects');
    expect(view?.textContent).not.toContain('metadata');
  });

  it('renders entry lists as tables with columns and row counts', () => {
    render(
      <StructuredResultView
        result={{
          entries: [
            { name: 'src', type: 'dir' },
            { name: 'calc.js', type: 'file' },
          ],
          truncated: false,
        }}
      />,
    );
    expect(screen.getByText('entries')).toBeInTheDocument();
    expect(screen.getByText('(2)')).toBeInTheDocument();
    expect(screen.getByRole('columnheader', { name: 'name' })).toBeInTheDocument();
    expect(screen.getByRole('columnheader', { name: 'type' })).toBeInTheDocument();
    expect(screen.getByRole('cell', { name: 'calc.js' })).toBeInTheDocument();
    expect(screen.getByRole('cell', { name: 'dir' })).toBeInTheDocument();
  });

  it('caps large tables and points at the raw result', () => {
    const tools = Array.from({ length: 12 }, (_, index) => ({
      name: `tool_${index}`,
      connector: 'Harness',
      description: 'fixture',
    }));
    const { container } = render(<StructuredResultView result={{ tools }} />);
    expect(container.querySelectorAll('tbody tr')).toHaveLength(12);
    const wide = Array.from({ length: 3 }, (_, index) => ({
      a: index, b: index, c: index, d: index, e: index, f: index, g: index, h: index,
    }));
    render(<StructuredResultView result={{ wide }} />);
    expect(screen.getByText(/more columns in the raw result/)).toBeInTheDocument();
  });

  it('renders nested objects as sections and long strings as scrollable text', () => {
    render(
      <StructuredResultView
        result={{
          data: { verdict: 'passed', summary: 'x'.repeat(500) },
          _artifactId: 'art-1',
          _truncated: true,
        }}
      />,
    );
    expect(screen.getByText('data')).toBeInTheDocument();
    expect(screen.getByText('verdict')).toBeInTheDocument();
    const pre = document.querySelector('.structured-result-text');
    expect(pre?.textContent?.length).toBeGreaterThan(400);
    // Artifact-bounding keys are as internal as the envelope.
    expect(document.querySelector('.structured-result')?.textContent).not.toContain('_artifactId');
  });

  it('handles envelope-only results without crashing', () => {
    render(
      <StructuredResultView
        result={{ schemaVersion: 'locallm.tool-envelope/1', status: 'success', isError: false, artifacts: [] }}
      />,
    );
    expect(screen.getByText(/Only envelope metadata/)).toBeInTheDocument();
  });
});
