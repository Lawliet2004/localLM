import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { ToolResultExtras } from './ToolResultExtras';
const fixture = vi.hoisted(() => ({ get: vi.fn(), preview: vi.fn() }));
vi.mock('../lib/api', () => ({ nativeAvailable: false, errorMessage: String, api: { getArtifact: fixture.get, previewExecutionArtifact: fixture.preview } }));
it('renders full weather data from a conversation-owned artifact with Fahrenheit and freshness', async () => {
  fixture.get.mockResolvedValue({ conversationId: 'chat', mimeType: 'application/json', content: JSON.stringify({
    location: { name: 'Example city' }, current: { temperatureC: 0, apparentTemperatureC: -1, humidityPercent: 80, windKph: 12, weatherCode: 61 },
    forecast: [{ date: '2026-10-16', minC: 0, maxC: 10, precipitationProbabilityPercent: 70 }], hourly: [],
    timezone: 'Asia/Tokyo', observedAt: '2026-10-16T12:00', fetchedAt: '2026-10-16T03:01:00Z', temperatureUnit: 'fahrenheit', cached: true,
  }) });
  render(<ToolResultExtras name="get_weather" result={{ _artifactId: 'full' }} conversationId="chat" />);
  expect(await screen.findByRole('region', { name: 'Weather forecast' })).toHaveTextContent('32.0°F');
  expect(screen.getByText(/Cached 2026-10-16T03:01:00Z/)).toBeInTheDocument();
  expect(screen.getByRole('table')).toHaveTextContent('70%');
});
it.each(['run_code', 'execute_command'])('previews %s output through its bound workspace and recorded content hash', async name => {
  fixture.preview.mockResolvedValue({ mimeType: 'text/plain', content: '<script>literal file content</script>' });
  render(<ToolResultExtras name={name} result={{ workspace: '/workspace', artifacts: [{ path: 'result.html', sha256: 'a'.repeat(64), bytes: 32 }] }} conversationId="chat" />);
  fireEvent.click(screen.getByRole('button', { name: 'Preview result.html' }));
  await waitFor(() => expect(fixture.preview).toHaveBeenCalledWith('/workspace', 'result.html', 'a'.repeat(64)));
  const source = await screen.findByLabelText('result.html source');
  expect(source).toHaveTextContent('<script>literal file content</script>');
  expect(document.querySelector('iframe')).toBeNull();
});
it('rejects a full tool artifact from another conversation', async () => {
  fixture.get.mockResolvedValue({ conversationId: 'other', mimeType: 'application/json', content: '{}' });
  render(<ToolResultExtras name="get_weather" result={{ _artifactId: 'foreign' }} conversationId="chat" />);
  expect(await screen.findByRole('alert')).toHaveTextContent('unavailable in this conversation');
});
