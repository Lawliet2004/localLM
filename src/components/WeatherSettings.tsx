import { useEffect, useState } from 'react';
import { api, errorMessage, nativeAvailable } from '../lib/api';
import type { WeatherConfig } from '../lib/types';

export function WeatherSettings() {
  const [config, setConfig] = useState<WeatherConfig>({ enabled: true, location: '', temperatureUnit: 'celsius' });
  const [busy, setBusy] = useState(nativeAvailable);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  useEffect(() => {
    if (!nativeAvailable) return;
    let disposed = false;
    api.getWeatherConfig().then(value => { if (!disposed) { setConfig(value); setLoaded(true); } }).catch(e => { if (!disposed) setError(errorMessage(e)); }).finally(() => { if (!disposed) setBusy(false); });
    return () => { disposed = true; };
  }, []);
  async function save() {
    setBusy(true); setError(''); setNotice('');
    try { await api.saveWeatherConfig(config); setNotice('Weather preferences saved.'); }
    catch (e) { setError(errorMessage(e)); } finally { setBusy(false); }
  }
  return <div className="settings-page">
    <div className="page-heading"><h1>Weather</h1><p>Current conditions and forecasts from Open-Meteo.</p></div>
    {!nativeAvailable && <p className="catalog-notice">Open the desktop application to save weather preferences.</p>}
    <form onSubmit={event => { event.preventDefault(); void save(); }}>
      <fieldset disabled={!nativeAvailable || busy || !loaded}>
        <section className="preference-section" aria-labelledby="weather-settings-heading">
          <div className="preference-section-heading"><h2 id="weather-settings-heading">Forecast</h2></div>
          <div className="preference-group">
            <div className="preference-row preference-toggle">
              <div className="preference-copy">
                <label htmlFor="weather-enabled">Weather tool</label>
                <p id="weather-enabled-desc">Available to the assistant in every conversation.</p>
              </div>
              <input id="weather-enabled" className="switch" type="checkbox" aria-label="Enable weather tool" aria-describedby="weather-enabled-desc" checked={config.enabled} onChange={event => { setNotice(''); setConfig(current => ({ ...current, enabled: event.target.checked })); }} />
            </div>
            <div className="preference-row">
              <div className="preference-copy">
                <label htmlFor="weather-units">Units</label>
                <p>Temperature units used in reports.</p>
              </div>
              <select id="weather-units" value={config.temperatureUnit} onChange={event => { setNotice(''); setConfig(current => ({ ...current, temperatureUnit: event.target.value as WeatherConfig['temperatureUnit'] })); }}><option value="celsius">Celsius (°C)</option><option value="fahrenheit">Fahrenheit (°F)</option></select>
            </div>
            <div className="preference-row preference-row-block">
              <div className="preference-copy">
                <label htmlFor="weather-location">Preferred location</label>
                <p>Optional default when a request omits its location. Device location is never requested.</p>
              </div>
              <input id="weather-location" maxLength={200} value={config.location} placeholder="City, region, country" onChange={event => { setNotice(''); setConfig(current => ({ ...current, location: event.target.value })); }} />
            </div>
            <div className="connector-actions">
              <button className="primary">{busy ? 'Saving…' : 'Save weather preferences'}</button>
            </div>
          </div>
        </section>
      </fieldset>
    </form>
    <p className="catalog-notice">Forecasts are cached for ten minutes; results keep their location, timezone, and observation time. <a href="https://open-meteo.com/" target="_blank" rel="noreferrer">Weather data by Open-Meteo</a> · <a href="https://creativecommons.org/licenses/by/4.0/" target="_blank" rel="noreferrer">CC BY 4.0</a> — the public API is for non-commercial use.</p>
    {error && <p className="error-banner" role="alert">{error}</p>}{notice && <p role="status">{notice}</p>}
  </div>;
}
