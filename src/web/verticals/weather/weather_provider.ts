import type { VerticalProvider } from '../../types';
export interface WeatherReport {
  sourceUrl: string;
  observedAt: string;
  timezone: string;
  location: { name: string; region?: string; country?: string; latitude: number; longitude: number };
  current: { temperatureC: number; apparentTemperatureC: number; humidityPercent: number; windKph: number };
  forecast: Array<{date: string; minC: number; maxC: number; precipitationProbabilityPercent: number}>;
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
    const url = `https://geocoding-api.open-meteo.com/v1/search?name=${encodeURIComponent(name)}&count=5&language=en&format=json`;
    const res = await fetch(url, {signal: AbortSignal.timeout(8000)});
    if (!res.ok) throw new Error(`Geocoding unavailable (HTTP ${res.status})`);
    const data = await res.json();
    const first = data.results?.[0];
    if (!first || !Number.isFinite(first.latitude) || !Number.isFinite(first.longitude)) throw new Error(`Could not resolve location: ${name}`);
    return {name: String(first.name).normalize('NFD').replace(/[\u0300-\u036f]/g, ''), region: first.admin1, country: first.country, lat: first.latitude, lon: first.longitude};
  }
  async execute(query: string): Promise<WeatherReport> {
    const coords = await this.geocode(this.extractLocation(query));
    const sourceUrl = `https://api.open-meteo.com/v1/forecast?latitude=${coords.lat}&longitude=${coords.lon}&current=temperature_2m,relative_humidity_2m,apparent_temperature,wind_speed_10m&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max&timezone=auto&forecast_days=3`;
    const res = await fetch(sourceUrl, {signal: AbortSignal.timeout(8000)});
    if (!res.ok) throw new Error(`Weather unavailable (HTTP ${res.status})`);
    const data = await res.json();
    const cur = data.current;
    if (!cur || ![cur.temperature_2m, cur.apparent_temperature, cur.relative_humidity_2m, cur.wind_speed_10m].every(Number.isFinite)) throw new Error('Weather response has missing measurements');
    const daily = data.daily || {};
    const forecast: WeatherReport['forecast'] = [];
    for (let i = 0; i < Math.min(daily.time?.length || 0, 3); i++) {
      const values = [daily.temperature_2m_min?.[i], daily.temperature_2m_max?.[i], daily.precipitation_probability_max?.[i]];
      if (values.every(Number.isFinite)) forecast.push({date: daily.time[i], minC: values[0], maxC: values[1], precipitationProbabilityPercent: values[2]});
    }
    return {sourceUrl, observedAt: cur.time, timezone: data.timezone,
      location: {name: coords.name, region: coords.region, country: coords.country, latitude: coords.lat, longitude: coords.lon},
      current: {temperatureC:cur.temperature_2m, apparentTemperatureC:cur.apparent_temperature, humidityPercent:cur.relative_humidity_2m, windKph:cur.wind_speed_10m}, forecast};
  }
  formatReportAsEvidence(report: WeatherReport): string {
    const loc = report.location, cur = report.current;
    return `Location: ${[loc.name,loc.region,loc.country].filter(Boolean).join(', ')}\nAs of ${report.observedAt} (${report.timezone}), Open-Meteo model data.\nCurrent Temperature: ${cur.temperatureC}°C (Feels like ${cur.apparentTemperatureC}°C)\nHumidity: ${cur.humidityPercent}%\nWind Speed: ${cur.windKph} km/h\n` + report.forecast.map(f => `${f.date}: High ${f.maxC}°C, Low ${f.minC}°C, Rain probability ${f.precipitationProbabilityPercent}%`).join('\n');
  }
}
