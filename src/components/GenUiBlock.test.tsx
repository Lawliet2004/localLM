import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { GenUiBlock } from './GenUiBlock';

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));

const valid =
  'root = Response([summary, stats])\n' +
  'summary = Markdown("All **3 files** downloaded.")\n' +
  'stats = Columns([Stat("Downloaded", "2.07 GB"), Stat("Errors", "0")])';

describe('GenUiBlock', () => {
  it('renders a valid program as structured UI', async () => {
    render(<GenUiBlock code={valid} />);
    expect(await screen.findByText('Downloaded')).toBeInTheDocument();
    expect(screen.getByText('2.07 GB')).toBeInTheDocument();
    expect(screen.getByText('Errors')).toBeInTheDocument();
  });

  it('toggles between rendered and source views', async () => {
    render(<GenUiBlock code={valid} />);
    expect(await screen.findByText('Downloaded')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Source' }));
    expect(screen.getByText(/root = Response/)).toBeInTheDocument();
    expect(screen.queryByText('Downloaded')).not.toBeInTheDocument();
  });

  it('shows a safe fallback and keeps source available for invalid input', async () => {
    render(<GenUiBlock code={'not a program at all'} />);
    expect(await screen.findByText(/Couldn’t render this UI/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Source' }));
    expect(screen.getByText(/not a program at all/)).toBeInTheDocument();
  });

  it('sends a follow-up when a continue_conversation button is clicked', async () => {
    const onFollowUp = vi.fn();
    const code = 'root = Response([b])\nb = Button("Compare benchmarks")';
    render(<GenUiBlock code={code} onFollowUp={onFollowUp} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Compare benchmarks' }));
    expect(onFollowUp).toHaveBeenCalledWith('Compare benchmarks');
  });

  it('handles interrupted programs and recovers for a subsequent valid answer', async () => {
    const partial = 'root = Response([t])\nt = Text("Partial';
    const { rerender } = render(<GenUiBlock code={partial} streaming />);
    expect(screen.queryByText(/root =/)).not.toBeInTheDocument();
    rerender(<GenUiBlock code={partial} streaming={false} />);
    expect(await screen.findByRole('status')).toHaveTextContent('incomplete');
    rerender(<GenUiBlock code={valid} streaming={false} />);
    expect(await screen.findByText('Downloaded')).toBeInTheDocument();
  });

  it('opens a link for open_url buttons', async () => {
    const open = vi.spyOn(window, 'open').mockImplementation(() => null);
    const code = 'root = Response([b])\nb = Button("Docs", { type: "open_url", url: "https://example.com/docs" })';
    render(<GenUiBlock code={code} />);
    fireEvent.click(await screen.findByRole('button', { name: 'Docs' }));
    expect(open).toHaveBeenCalledWith('https://example.com/docs', '_blank', 'noopener,noreferrer');
    open.mockRestore();
  });
});
