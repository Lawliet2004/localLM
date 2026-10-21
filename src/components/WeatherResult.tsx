import type { WeatherReport } from '../web/verticals/weather/weather_provider';
import { weatherCondition, weatherTemperature } from '../web/verticals/weather/weather_provider';

export function asWeatherReport(value: unknown): WeatherReport | null {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const report = value as WeatherReport;
  if (!report.location || typeof report.location.name !== 'string' || !report.current || !Array.isArray(report.forecast) || typeof report.timezone !== 'string' || typeof report.observedAt !== 'string') return null;
  if (![report.current.temperatureC, report.current.apparentTemperatureC, report.current.humidityPercent, report.current.windKph].every(value => typeof value === 'number' && Number.isFinite(value))) return null;
  if (!report.forecast.every(day => typeof day.date === 'string' && [day.minC, day.maxC, day.precipitationProbabilityPercent].every(value => typeof value === 'number' && Number.isFinite(value)))) return null;
  return report;
}

export function WeatherResult({ report }: { report: WeatherReport }) {
  const unit = report.temperatureUnit === 'fahrenheit' ? 'fahrenheit' : 'celsius';
  const temperature = (value: number) => `${weatherTemperature(value, unit).toFixed(1)}${unit === 'fahrenheit' ? '°F' : '°C'}`;
  const hourly = (report.hourly || []).slice(0, 24);
  return <section className="weather-result" aria-label="Weather forecast">
    <h3>{[report.location.name, report.location.region, report.location.country].filter(Boolean).join(', ')}</h3>
    <p className="weather-current">{temperature(report.current.temperatureC)} <span>{weatherCondition(report.current.weatherCode)}</span></p>
    <p>Feels like {temperature(report.current.apparentTemperatureC)} · Humidity {report.current.humidityPercent}% · Wind {report.current.windKph} km/h{typeof report.current.precipitationMm === 'number' ? ` · Rain ${report.current.precipitationMm} mm` : ''}</p>
    <p className="weather-freshness">As of {report.observedAt.replace('T', ' ')} ({report.timezone}) · {report.cached ? 'Cached' : 'Fetched'} {report.fetchedAt || 'at an unavailable time'}</p>
    <div className="weather-table"><table><caption>Daily forecast</caption><thead><tr><th>Date</th><th>Low</th><th>High</th><th>Rain chance</th></tr></thead><tbody>{report.forecast.map(day => <tr key={day.date}><th scope="row">{day.date}</th><td>{temperature(day.minC)}</td><td>{temperature(day.maxC)}</td><td>{day.precipitationProbabilityPercent}%</td></tr>)}</tbody></table></div>
    {hourly.length > 0 && <details><summary>Hourly forecast · {report.timezone}</summary><div className="weather-table"><table><thead><tr><th>Local time</th><th>Temperature</th><th>Feels like</th><th>Rain chance</th><th>Humidity</th><th>Wind</th></tr></thead><tbody>{hourly.map(hour => <tr key={hour.time}><th scope="row">{hour.time.replace('T', ' ')}</th><td>{temperature(hour.temperatureC)}</td><td>{temperature(hour.apparentTemperatureC)}</td><td>{hour.precipitationProbabilityPercent}%</td><td>{hour.humidityPercent}%</td><td>{hour.windKph} km/h</td></tr>)}</tbody></table></div></details>}
    <p className="weather-freshness"><a href="https://open-meteo.com/" target="_blank" rel="noreferrer">Open-Meteo model data · CC BY 4.0</a></p>
  </section>;
}
