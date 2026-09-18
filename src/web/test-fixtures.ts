import { vi } from 'vitest';
import { HttpFetcher } from './fetch/http_fetcher';
/** Explicit HTTP boundary fixture. Production providers never return fixture data. */
export function installWebFixtures() {
  vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes('geocoding-api.open-meteo')) return Response.json({results:[{name:'Ranaghat',admin1:'West Bengal',country:'India',latitude:23.18,longitude:88.58}]});
    if (url.includes('api.open-meteo')) return Response.json({timezone:'Asia/Kolkata', current:{time:'2026-09-16T12:00',temperature_2m:31.2,apparent_temperature:36,relative_humidity_2m:78,wind_speed_10m:12},daily:{time:['2026-09-16'],temperature_2m_min:[26],temperature_2m_max:[32],precipitation_probability_max:[65]}});
    return new Response('Unavailable', {status:404});
  }));
  vi.spyOn(HttpFetcher.prototype, 'fetch').mockImplementation(async (url) => {
    const res = await fetch(url);
    return {url,finalUrl:url,success:res.ok,status:res.status,body:await res.text(),durationMs:0};
  });
}
