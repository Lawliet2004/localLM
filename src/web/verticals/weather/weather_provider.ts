import type { VerticalProvider } from '../../types';
export interface WeatherReport {
  fetchedAt: string;
  cached?: boolean;
  temperatureUnit: 'celsius' | 'fahrenheit';
  sourceUrl: string;
  observedAt: string;
  timezone: string;
  units: { temperature: '°C'; windSpeed: 'km/h'; humidity: '%'; precipitationProbability: '%' };
  location: { name: string; region?: string; country?: string; latitude: number; longitude: number };
  current: { temperatureC: number; apparentTemperatureC: number; humidityPercent: number; windKph: number; weatherCode?: number; precipitationMm?: number };
  hourly: Array<{time: string; temperatureC: number; apparentTemperatureC: number; precipitationProbabilityPercent: number; humidityPercent: number; windKph: number; weatherCode: number}>;
  forecast: Array<{date: string; minC: number; maxC: number; precipitationProbabilityPercent: number}>;
}
export const weatherTemperature = (celsius: number, unit: WeatherReport['temperatureUnit']) => unit === 'fahrenheit' ? celsius * 9 / 5 + 32 : celsius;
export function weatherCondition(code?: number): string {
  if (code === 0) return 'Clear sky';
  if (code !== undefined && code >= 1 && code <= 3) return ['Mainly clear', 'Partly cloudy', 'Overcast'][code - 1];
  if (code === 45 || code === 48) return 'Fog';
  if (code !== undefined && code >= 51 && code <= 57) return 'Drizzle';
  if (code !== undefined && code >= 61 && code <= 67) return 'Rain';
  if (code !== undefined && code >= 71 && code <= 77) return 'Snow';
  if (code !== undefined && code >= 80 && code <= 82) return 'Rain showers';
  if (code === 85 || code === 86) return 'Snow showers';
  if (code === 95 || code === 96 || code === 99) return 'Thunderstorm';
  return 'Conditions unavailable';
}

export function validateWeatherRequest(location: unknown, forecastDays: unknown = 3): { location: string; forecastDays: number } {
  if (typeof location !== 'string' || !location.trim() || location.length > 200) {
    throw new Error('Weather requires an explicit location of 1–200 characters.');
  }
  if (typeof forecastDays !== 'number' || !Number.isInteger(forecastDays) || forecastDays < 1 || forecastDays > 7) {
    throw new Error('forecastDays must be an integer from 1 to 7.');
  }
  return { location: location.trim(), forecastDays };
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`Weather response has invalid ${label}.`);
  return value as Record<string, unknown>;
}

function text(value: unknown, label: string): string {
  if (typeof value !== 'string' || !value.trim() || value.length > 500) throw new Error(`Weather response has missing ${label}.`);
  return value.trim();
}

function measurement(value: unknown, label: string, min = -Infinity, max = Infinity): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < min || value > max) {
    throw new Error(`Weather response has missing or invalid ${label}.`);
  }
  return value;
}

async function requestJson(url: string, label: string): Promise<Record<string, unknown>> {
  const res = await fetch(url, { signal: AbortSignal.timeout(8000), redirect: 'error' });
  if (!res.ok) throw new Error(`${label} unavailable (HTTP ${res.status}).`);
  const reader = res.body?.getReader();
  if (!reader) throw new Error(`${label} returned an empty response.`);
  const chunks: Uint8Array[] = []; let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > 1024 * 1024) throw new Error(`${label} response exceeds 1 MiB.`);
      chunks.push(value);
    }
  } finally { await reader.cancel(); }
  const bytes = new Uint8Array(size); let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
  const data = record(JSON.parse(new TextDecoder().decode(bytes)), label);
  if (data.error === true) throw new Error(`${label} unavailable: ${typeof data.reason === 'string' ? data.reason.slice(0, 300) : 'provider error'}.`);
  return data;
}

export class WeatherProvider implements VerticalProvider<string, WeatherReport> {
  extractLocation(query: string): string {
    const match = query.match(/(?:weather(?:\s+report)?|temperature|forecast)(?:\s+(?:in|for|at))\s+([^?!]+)/i)
      || query.match(/(?:in|for|at)\s+([^?!]+)/i)
      || query.match(/^([^?!]+)\s+weather/i);
    const location = match?.[1].replace(/\b(today|tomorrow|now|tonight|this week|current)\b/gi, '').replace(/[.]+$/, '').trim();
    if (!location) throw new Error('Please specify a location for weather.');
    return location;
  }
  async geocode(name: string) {
    const parts = name.split(',').map(part => part.trim());
    if (parts.some(part => !part) || parts.length > 3) throw new Error(`Could not resolve all location qualifiers: ${name}. Try City, country or City, state.`);
    const url = `https://geocoding-api.open-meteo.com/v1/search?name=${encodeURIComponent(parts[0])}&count=10&language=en&format=json`;
    const data = await requestJson(url, 'Geocoding');
    if (!Array.isArray(data.results) || !data.results.length) throw new Error(`Could not resolve location: ${name}. Try City, country or City, state.`);
    const resolved = data.results.map(value => record(value, 'location'));
    const normalizeName = (value: string) => value.normalize('NFKD').replace(/\p{M}/gu, '').toLocaleLowerCase('en');
    const exact = resolved.filter(candidate => typeof candidate.name === 'string' && normalizeName(candidate.name) === normalizeName(parts[0]));
    const candidates = (exact.length ? exact : resolved).filter(candidate => parts.slice(1).every(part =>
      [candidate.country, candidate.country_code, candidate.admin1, candidate.admin2].some(value => typeof value === 'string' && value.toLocaleLowerCase('en') === part.toLocaleLowerCase('en'))));
    if (!candidates.length) {
      throw new Error(`Could not resolve all location qualifiers: ${name}. Try City, country or City, state.`);
    }
    const distinct = new Map(candidates.map(candidate => [`${candidate.name}|${candidate.admin1}|${candidate.country}`, candidate]));
    if (distinct.size > 1) throw new Error(`Ambiguous location. Specify a city and region or country: ${[...distinct.values()].slice(0, 5).map(candidate => [candidate.name, candidate.admin1, candidate.country].filter(Boolean).join(', ')).join('; ')}.`);
    const first = candidates[0];
    return {
      name: text(first.name, 'location name'),
      region: typeof first.admin1 === 'string' ? first.admin1 : undefined,
      country: typeof first.country === 'string' ? first.country : undefined,
      lat: measurement(first.latitude, 'latitude', -90, 90),
      lon: measurement(first.longitude, 'longitude', -180, 180),
    };
  }
  async execute(query: string): Promise<WeatherReport> {
    return this.executeLocation(this.extractLocation(query));
  }
  async executeLocation(location: unknown, forecastDays: unknown = 3, temperatureUnit: unknown = 'celsius'): Promise<WeatherReport> {
    const request = validateWeatherRequest(location, forecastDays);
    if (temperatureUnit !== 'celsius' && temperatureUnit !== 'fahrenheit') throw new Error('Temperature unit must be celsius or fahrenheit.');
    const coords = await this.geocode(request.location);
    const sourceUrl = `https://api.open-meteo.com/v1/forecast?latitude=${coords.lat}&longitude=${coords.lon}&current=temperature_2m,relative_humidity_2m,apparent_temperature,wind_speed_10m,weather_code,precipitation&hourly=temperature_2m,apparent_temperature,relative_humidity_2m,precipitation_probability,wind_speed_10m,weather_code&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max&temperature_unit=celsius&wind_speed_unit=kmh&timezone=auto&forecast_days=${request.forecastDays}`;
    const data = await requestJson(sourceUrl, 'Weather');
    const cur = record(data.current, 'current conditions');
    const observedAt = text(cur.time, 'current timestamp');
    if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(observedAt) || !Number.isFinite(Date.parse(observedAt))) throw new Error('Weather response has invalid current timestamp.');
    const timezone = text(data.timezone, 'timezone');
    try { new Intl.DateTimeFormat('en', { timeZone: timezone }); } catch { throw new Error('Weather response has invalid timezone.'); }
    const daily = record(data.daily, 'daily forecast');
    if (!Array.isArray(daily.time) || !daily.time.length) throw new Error('Weather response has missing forecast dates.');
    const min = daily.temperature_2m_min, max = daily.temperature_2m_max, rain = daily.precipitation_probability_max;
    if (![min, max, rain].every(Array.isArray)) throw new Error('Weather response has missing forecast measurements.');
    const forecast: WeatherReport['forecast'] = [];
    for (let i = 0; i < Math.min(daily.time.length, request.forecastDays); i++) {
      const date = text(daily.time[i], 'forecast date');
      if (!/^\d{4}-\d{2}-\d{2}$/.test(date) || !Number.isFinite(Date.parse(date))) throw new Error('Weather response has invalid forecast date.');
      const minC = measurement((min as unknown[])[i], 'minimum temperature');
      const maxC = measurement((max as unknown[])[i], 'maximum temperature', minC);
      forecast.push({date, minC, maxC, precipitationProbabilityPercent: measurement((rain as unknown[])[i], 'rain probability', 0, 100)});
    }
    const hourly: WeatherReport['hourly'] = [];
    if (data.hourly) {
      const hours = record(data.hourly, 'hourly forecast');
      if (!Array.isArray(hours.time)) throw new Error('Weather response has missing hourly times.');
      for (let i = 0; i < hours.time.length && hourly.length < 48; i++) {
        const time = text(hours.time[i], 'hourly timestamp');
        if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(time)) throw new Error('Weather response has invalid hourly time.');
        if (time < observedAt.slice(0, 13) + ':00') continue;
        const at = (key: string, min = -Infinity, max = Infinity) => measurement((hours[key] as unknown[] | undefined)?.[i], key, min, max);
        hourly.push({ time, temperatureC: at('temperature_2m'), apparentTemperatureC: at('apparent_temperature'), humidityPercent: at('relative_humidity_2m', 0, 100), precipitationProbabilityPercent: at('precipitation_probability', 0, 100), windKph: at('wind_speed_10m', 0), weatherCode: at('weather_code', 0, 99) });
      }
    }
    return {sourceUrl, observedAt, timezone, fetchedAt: new Date().toISOString(), temperatureUnit, hourly, units: {temperature: '°C', windSpeed: 'km/h', humidity: '%', precipitationProbability: '%'},
      location: {name: coords.name, region: coords.region, country: coords.country, latitude: coords.lat, longitude: coords.lon},
      current: {temperatureC: measurement(cur.temperature_2m, 'temperature'), apparentTemperatureC: measurement(cur.apparent_temperature, 'apparent temperature'), humidityPercent: measurement(cur.relative_humidity_2m, 'humidity', 0, 100), windKph: measurement(cur.wind_speed_10m, 'wind speed', 0), weatherCode: cur.weather_code === undefined ? undefined : measurement(cur.weather_code, 'conditions', 0, 99), precipitationMm: cur.precipitation === undefined ? undefined : measurement(cur.precipitation, 'precipitation', 0)}, forecast};
  }
  formatReportAsEvidence(report: WeatherReport): string {
    const loc = report.location, cur = report.current;
    const unit = report.temperatureUnit === 'fahrenheit' ? '°F' : '°C';
    const temp = (value: number) => `${weatherTemperature(value, report.temperatureUnit).toFixed(1)}${unit}`;
    return `Location: ${[loc.name,loc.region,loc.country].filter(Boolean).join(', ')}\nAs of ${report.observedAt} (${report.timezone}), Open-Meteo model data.\nCurrent Temperature: ${temp(cur.temperatureC)} (Feels like ${temp(cur.apparentTemperatureC)})\nHumidity: ${cur.humidityPercent}%\nWind Speed: ${cur.windKph} km/h\n` + report.forecast.map(f => `${f.date}: High ${temp(f.maxC)}, Low ${temp(f.minC)}, Rain probability ${f.precipitationProbabilityPercent}%`).join('\n');
  }
}
