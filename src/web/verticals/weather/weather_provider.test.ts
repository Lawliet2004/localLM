import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { installWebFixtures } from '../../test-fixtures';
import { WeatherProvider } from './weather_provider';

describe('Weather Vertical Provider', () => {
  beforeEach(installWebFixtures);
  afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); });
  const provider = new WeatherProvider();

  it('extracts location from weather queries', () => {
    expect(provider.extractLocation('What is the weather in Ranaghat today?')).toBe('Ranaghat');
    expect(provider.extractLocation('Current temperature in Tokyo')).toBe('Tokyo');
    expect(provider.extractLocation('Weather report for London UK tonight')).toBe('London UK');
    expect(provider.extractLocation('What is the weather of Kolkata?')).toBe('Kolkata');
    expect(provider.extractLocation('What is the current weather report data for Kolkata?')).toBe('Kolkata');
    expect(provider.extractLocation("What is today's weather report for Kolkata?\n- Max temperature\n- Rain chance")).toBe('Kolkata');
  });

  it('returns compact structured weather report', async () => {
    const report = await provider.execute('weather in Ranaghat today');
    expect(report.location.name).toBe('Ranaghat');
    expect(report.current.temperatureC).toBeDefined();
    expect(report.forecast.length).toBeGreaterThan(0);

    const evidenceText = provider.formatReportAsEvidence(report);
    expect(evidenceText).toContain('Ranaghat');
    expect(evidenceText).toContain('Temperature:');
    // Compact: under 100 words
    expect(evidenceText.split(/\s+/).length).toBeLessThan(100);
  }, 15000);

  it('resolves the common Kokata spelling to Kolkata before geocoding', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch');
    await provider.execute('What is the weather of Kokata?');
    expect(fetchSpy.mock.calls[0][0]).toContain('name=Kolkata');
    await provider.geocode('Kokata, Bolivia');
    expect(fetchSpy.mock.calls[2][0]).toContain('name=Kokata%2C%20Bolivia');
  });
});
