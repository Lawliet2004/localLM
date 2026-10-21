import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ScreenshotPreview } from './ScreenshotPreview';
const f = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: false, errorMessage: String, api: { getArtifact: f.get } }));
describe('screenshot preview', () => {
  it('loads an image only when opened and never renders base64 as text', async () => {
    f.get.mockReset().mockResolvedValue({ conversationId: 'one', mimeType: 'image/png', content: 'data:image/png;base64,abc' });
    const { container } = render(<ScreenshotPreview id="art_one" conversationId="one" />);
    expect(f.get).not.toHaveBeenCalled();
    const details = container.querySelector('details')!;
    details.open = true; fireEvent(details, new Event('toggle'));
    await act(async () => {});
    expect(await screen.findByAltText('Automation screenshot')).toHaveAttribute('src', 'data:image/png;base64,abc');
    expect(container.textContent).not.toContain('base64');
  });
  it('rejects images from another conversation', async () => {
    f.get.mockReset().mockResolvedValue({ conversationId: 'other', mimeType: 'image/png', content: 'data:image/png;base64,abc' });
    const { container } = render(<ScreenshotPreview id="art_other" conversationId="one" />);
    const details = container.querySelector('details')!;
    details.open = true; fireEvent(details, new Event('toggle'));
    expect(await screen.findByRole('alert')).toHaveTextContent('unavailable in this conversation');
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
  });
});
