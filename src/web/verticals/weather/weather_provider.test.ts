import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { installWebFixtures } from '../../test-fixtures';
import { WeatherProvider, weatherTemperature, weatherCondition } from './weather_provider';

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

  it('passes an explicit location directly and bounds forecast output', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(Response.json({results:[{name:'Current River', country:'Canada', latitude:48.4, longitude:-89.2}]}))
      .mockResolvedValueOnce(Response.json(validWeather()));
    vi.stubGlobal('fetch', fetchMock);
    const report = await provider.executeLocation('Current River', 2);
    expect(new URL(fetchMock.mock.calls[0][0]).searchParams.get('name')).toBe('Current River');
    const url = new URL(report.sourceUrl);
    expect(url.searchParams.get('forecast_days')).toBe('2');
    expect(url.searchParams.get('temperature_unit')).toBe('celsius');
    expect(url.searchParams.get('wind_speed_unit')).toBe('kmh');
    expect(report.forecast).toHaveLength(2);
    expect(report.units).toEqual({temperature:'°C', windSpeed:'km/h', humidity:'%', precipitationProbability:'%'});
    expect(report.observedAt).toBe('2026-10-15T12:00');
    expect(report.timezone).toBe('Asia/Kolkata');
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it.each([undefined, null, '', '   ', 42, {}, 'a'.repeat(201)])('rejects invalid location before HTTP: %j', async location => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    await expect(provider.executeLocation(location)).rejects.toThrow('explicit location');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it.each([null, '3', 0, 8, -1, 1.5, NaN, Infinity])('rejects invalid forecast length before HTTP: %j', async days => {
    const fetchMock = vi.fn();
    vi.stubGlobal('fetch', fetchMock);
    await expect(provider.executeLocation('Tokyo', days)).rejects.toThrow('integer from 1 to 7');
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('does not guess when geocoding returns no location', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({results:[]})));
    await expect(provider.executeLocation('Unknown City')).rejects.toThrow('Could not resolve location');
    expect(fetch).toHaveBeenCalledTimes(1);
  });

  it('rejects invalid coordinates and conflicting country qualifiers', async () => {
    for (const [location, result] of [
      ['Tokyo', {name:'Tokyo', latitude:200, longitude:139}],
      ['Paris, Texas, France', {name:'Paris', admin1:'Texas', country:'United States', country_code:'US', latitude:33.6, longitude:-95.5}],
    ] as const) {
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({results:[result]})));
      await expect(provider.executeLocation(location)).rejects.toThrow();
      expect(fetch).toHaveBeenCalledTimes(1);
    }
  });

  it('reports geocoding and forecast HTTP failures', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', {status:429})));
    await expect(provider.executeLocation('Tokyo')).rejects.toThrow('Geocoding unavailable (HTTP 429)');
    vi.stubGlobal('fetch', vi.fn()
      .mockResolvedValueOnce(Response.json({results:[{name:'Tokyo', latitude:35.7, longitude:139.7}]}))
      .mockResolvedValueOnce(new Response('', {status:503})));
    await expect(provider.executeLocation('Tokyo')).rejects.toThrow('Weather unavailable (HTTP 503)');
  });
  it('converts temperatures, preserves hourly local time and reports conditions', async () => {
    const weather = { ...validWeather(), current: { ...validWeather().current, weather_code: 61, precipitation: 1.2 },
      hourly: { time: ['2026-10-15T11:00', '2026-10-15T12:00'], temperature_2m: [20, 0], apparent_temperature: [19, -1], relative_humidity_2m: [80, 90], precipitation_probability: [50, 70], wind_speed_10m: [12, 15], weather_code: [3, 61] } };
    vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(Response.json({ results: [{ name: 'Tokyo', country: 'Japan', latitude: 35.7, longitude: 139.7 }] })).mockResolvedValueOnce(Response.json(weather)));
    const report = await provider.executeLocation('Tokyo, Japan', 3, 'fahrenheit');
    expect(report.hourly).toHaveLength(1);
    expect(report.hourly[0].time).toBe('2026-10-15T12:00');
    expect(report.current.precipitationMm).toBe(1.2);
    expect(report.fetchedAt).toMatch(/^\d{4}-/);
    expect(provider.formatReportAsEvidence(report)).toContain('88.2°F');
    expect(weatherTemperature(0, 'fahrenheit')).toBe(32);
    expect(weatherTemperature(100, 'fahrenheit')).toBe(212);
    expect(weatherCondition(report.current.weatherCode)).toBe('Rain');
  });
  it('reports ambiguous cities and selects a matching country qualifier', async () => {
    const results = [{ name: 'Paris', admin1: 'Texas', country: 'United States', latitude: 33.6, longitude: -95.5 }, { name: 'Paris', admin1: 'Île-de-France', country: 'France', latitude: 48.8, longitude: 2.3 }];
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(Response.json({ results })));
    await expect(provider.executeLocation('Paris')).rejects.toThrow('Ambiguous location');
    vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(Response.json({ results })).mockResolvedValueOnce(Response.json(validWeather())));
    expect((await provider.executeLocation('Paris, France')).location.country).toBe('France');
  });
  it('prefers an exact city match over fuzzy airports and similarly named places', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(Response.json({ results: [
      { name: 'Tokyo', admin1: 'Tokyo', country: 'Japan', latitude: 35.7, longitude: 139.7 },
      { name: 'Tokyo Heliport', admin1: 'Chiba', country: 'Japan', latitude: 35.6, longitude: 139.8 },
    ] })).mockResolvedValueOnce(Response.json(validWeather())));
    expect((await provider.executeLocation('Tokyo, Japan')).location.name).toBe('Tokyo');
  });

  it.each([
    { ...validWeather(), current: { ...validWeather().current, temperature_2m: null } },
    { ...validWeather(), current: { ...validWeather().current, relative_humidity_2m: 101 } },
    { ...validWeather(), current: { ...validWeather().current, time: '' } },
    { ...validWeather(), timezone: undefined },
    { ...validWeather(), daily: {} },
    { ...validWeather(), daily: { ...validWeather().daily, precipitation_probability_max: [null] } },
    { ...validWeather(), daily: { ...validWeather().daily, temperature_2m_min: [50, 50, 50] } },
    { error: true, reason: 'Service unavailable' },
    null,
  ])('rejects missing or invalid provider measurements/metadata: %#', async weather => {
    vi.stubGlobal('fetch', vi.fn()
      .mockResolvedValueOnce(Response.json({results:[{name:'Tokyo', latitude:35.7, longitude:139.7}]}))
      .mockResolvedValueOnce(Response.json(weather)));
    await expect(provider.executeLocation('Tokyo')).rejects.toThrow();
  });
});

function validWeather() {
  return {
    timezone:'Asia/Kolkata',
    current:{time:'2026-10-15T12:00', temperature_2m:31.2, apparent_temperature:36, relative_humidity_2m:78, wind_speed_10m:12},
    daily:{time:['2026-10-15','2026-10-16','2026-10-17'], temperature_2m_min:[26,25,24], temperature_2m_max:[32,31,30], precipitation_probability_max:[65,50,40]},
  };
}
