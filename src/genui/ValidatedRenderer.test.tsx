import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { ValidatedRenderer } from './ValidatedRenderer';
import { GenUiSurface } from './GenUiSurface';
import { VisualErrorBoundary } from './VisualErrorBoundary';

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));

const sample = 'root = Response([Heading("Sales"), Grid([Stat("Jan", "10"), Stat("Feb", "15"), Stat("Mar", "25")], 3), Table(["Month", "Sales"], [["Jan",10],["Feb",15],["Mar",25]])])';

describe('official Renderer smoke fixtures', () => {
  it('renders headings, responsive layouts, metrics and exact supplied table values', () => {
    render(<ValidatedRenderer response={sample} />);
    expect(screen.getByRole('heading', { name: 'Sales' })).toBeInTheDocument();
    expect(screen.getAllByRole('cell').map(cell => cell.textContent)).toEqual(['Jan', '10', 'Feb', '15', 'Mar', '25']);
  });
  it('streams root-first references without exposing raw syntax', () => {
    const { rerender } = render(<ValidatedRenderer response={'root = Response([t])\nt = Text("Part'} isStreaming />);
    expect(screen.queryByText(/root =/)).not.toBeInTheDocument();
    rerender(<ValidatedRenderer response={'root = Response([t])\nt = Text("Complete")'} isStreaming={false} />);
    expect(screen.getByText('Complete')).toBeInTheDocument();
  });
  it('disables follow-ups while streaming and enables them at completion', () => {
    const onAction = vi.fn();
    const source = 'root = Response([Button("Explain further")])';
    const { rerender } = render(<ValidatedRenderer response={source} isStreaming onAction={onAction} />);
    expect(screen.getByRole('button', { name: 'Explain further' })).toBeDisabled();
    rerender(<ValidatedRenderer response={source} isStreaming={false} onAction={onAction} />);
    fireEvent.click(screen.getByRole('button', { name: 'Explain further' }));
    expect(onAction).toHaveBeenCalledTimes(1);
  });
  it('blocks invalid properties in both chat foundation and canvas', () => {
    render(<GenUiSurface code={'root = Response([Progress("Fake", 999)])'} />);
    expect(screen.getByRole('status')).toHaveTextContent('Invalid or oversized Progress');
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument();
  });
  it('does not fetch generated images or render unsafe Markdown links', () => {
    const markdown = '[bad](javascript:alert(1)) ![secret](https://tracker.example/pixel) <script>alert(1)</script>';
    render(<ValidatedRenderer response={`root = Response([Markdown(${JSON.stringify(markdown)})])`} />);
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    expect(document.querySelector('script')).toBeNull();
  });
  it('reuses the shared syntax-highlighted code frame without executing source', () => {
    render(<ValidatedRenderer response={'root = Response([CodeBlock("print(1)", "python")])'} />);
    expect(screen.getByRole('button', { name: 'Copy code block' })).toBeInTheDocument();
    expect(document.querySelector('code.language-python')).not.toBeNull();
  });
  it('contains component exceptions and recovers when the program changes', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    function Broken(): never { throw new Error('test render failure'); }
    const { rerender } = render(<VisualErrorBoundary resetKey="broken"><Broken /></VisualErrorBoundary>);
    expect(screen.getByRole('status')).toHaveTextContent('Couldn’t display this UI');
    rerender(<VisualErrorBoundary resetKey="fixed"><p>Recovered</p></VisualErrorBoundary>);
    expect(screen.getByText('Recovered')).toBeInTheDocument();
    consoleError.mockRestore();
  });
});
