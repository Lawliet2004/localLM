import { afterEach, expect, it, vi } from 'vitest';
import { WeatherProvider } from './weather/weather_provider';
import { CurrencyProvider } from './currency/currency_provider';
afterEach(() => vi.unstubAllGlobals());
it('never fabricates weather or exchange rates while offline', async () => {
  vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('offline')));
  await expect(new WeatherProvider().execute('weather in Ranaghat today')).rejects.toThrow();
  await expect(new CurrencyProvider().execute('100 USD to EUR')).rejects.toThrow();
});
it('requires an explicit weather location', () => {
  expect(() => new WeatherProvider().extractLocation('weather today')).toThrow();
});
