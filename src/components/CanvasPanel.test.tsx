import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { CanvasPanel } from './CanvasPanel';
import type { CanvasArtifact } from '../lib/canvas';

vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));

const openuiArtifact: CanvasArtifact = {
  id: 'a1:0', groupKey: 'openui:dashboard', title: 'Dashboard', kind: 'openui', language: 'openui',
  code: 'root = Response([s])\ns = Stat("Total", "42")', messageId: 'a1', version: 2,
};
const codeArtifact: CanvasArtifact = {
  id: 'a2:0', groupKey: 'code:python snippet', title: 'python snippet', kind: 'code', language: 'python',
  code: 'print(1)', messageId: 'a2', version: 1,
};

describe('CanvasPanel', () => {
  it('renders an OpenUI artifact live and offers a Source view', async () => {
    render(<CanvasPanel artifact={openuiArtifact} versionCount={2} onVersion={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getByRole('complementary')).toHaveAccessibleName('Canvas: Dashboard');
    expect(await screen.findByText('Total')).toBeInTheDocument();
    expect(screen.getByText('42')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Source' }));
    expect(screen.getByText(/root = Response/)).toBeInTheDocument();
  });

  it('pages between artifact versions', () => {
    const onVersion = vi.fn();
    render(<CanvasPanel artifact={openuiArtifact} versionCount={3} onVersion={onVersion} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Previous version' }));
    expect(onVersion).toHaveBeenCalledWith(1);
    fireEvent.click(screen.getByRole('button', { name: 'Next version' }));
    expect(onVersion).toHaveBeenCalledWith(3);
  });

  it('shows plain code as source only and closes', () => {
    const onClose = vi.fn();
    render(<CanvasPanel artifact={codeArtifact} versionCount={1} onVersion={vi.fn()} onClose={onClose} />);
    expect(screen.getByText('print(1)')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Previous version' })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Close canvas' }));
    expect(onClose).toHaveBeenCalled();
  });
});
