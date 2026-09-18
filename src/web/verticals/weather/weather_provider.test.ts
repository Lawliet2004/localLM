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
});
