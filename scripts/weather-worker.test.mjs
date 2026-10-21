import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';

// Run the shipped artifact with a strict HTTP boundary. Any search, model,
// or unexpected request fails; no live weather service is needed.
const preload = `
let calls = 0;
globalThis.fetch = async input => {
  calls++;
  const url = new URL(String(input));
  if (url.hostname === 'geocoding-api.open-meteo.com') {
    return Response.json({results:url.searchParams.get('name') === 'Missing' ? [] : [
      {name:'Ranaghat',admin1:'West Bengal',country:'India',latitude:23.18,longitude:88.58}
    ]});
  }
  if (url.hostname !== 'api.open-meteo.com') throw new Error('Unexpected network request: ' + url.href);
  return Response.json({timezone:'Asia/Kolkata',
    current:{time:'2026-10-15T12:00',temperature_2m:31.2,apparent_temperature:36,relative_humidity_2m:78,wind_speed_10m:12},
    daily:{time:['2026-10-15','2026-10-16','2026-10-17'],temperature_2m_min:[26,25,24],temperature_2m_max:[32,31,30],precipitation_probability_max:[65,50,40]}
  });
};
process.on('exit', () => process.stderr.write('HTTP_CALLS=' + calls));
`;

function worker(input) {
  return spawnSync(process.execPath, [
    '--import', `data:text/javascript,${encodeURIComponent(preload)}`,
    resolve('src-tauri/resources/web/worker.mjs'),
  ], {input:JSON.stringify({action:'weather', ...input}), encoding:'utf8', timeout:15000});
}

test('bundled weather action returns bounded source data without search, storage, or inference', () => {
  const output = worker({location:'Ranaghat',forecastDays:2,
    config:{searchProvider:'invalid',enabled:false}, databasePath:'/dev/null/unavailable.sqlite',
    localModel:{baseUrl:'https://inference.invalid',modelName:'unused'},
  });
  assert.equal(output.status, 0, output.stderr);
  const report = JSON.parse(output.stdout);
  assert.equal(report.location.name, 'Ranaghat');
  assert.equal(report.current.temperatureC, 31.2);
  assert.equal(report.timezone, 'Asia/Kolkata');
  assert.equal(report.forecast.length, 2);
  assert.equal(new URL(report.sourceUrl).searchParams.get('forecast_days'), '2');
  assert.match(report.evidence, /Open-Meteo model data/);
  assert.match(output.stderr, /HTTP_CALLS=2/);
});

test('bundled weather rejects invalid input before network access', () => {
  for (const input of [{}, {location:' '}, {location:'Tokyo',forecastDays:8}, {location:'Tokyo',forecastDays:'3'}]) {
    const output = worker(input);
    assert.notEqual(output.status, 0);
    assert.equal(output.stdout, '');
    assert.match(output.stderr, /HTTP_CALLS=0/);
  }
});

test('bundled weather reports unresolved locations without a forecast request', () => {
  const output = worker({location:'Missing'});
  assert.notEqual(output.status, 0);
  assert.match(output.stderr, /Could not resolve location/);
  assert.match(output.stderr, /HTTP_CALLS=1/);
});
test('bundled weather persists a bounded fresh report across worker restarts', () => {
  const directory = mkdtempSync(resolve(tmpdir(), 'locallm-weather-'));
  try {
    const request = { location: 'Ranaghat', temperatureUnit: 'fahrenheit', weatherCachePath: resolve(directory, 'weather.sqlite') };
    const first = worker(request), second = worker(request);
    assert.equal(first.status, 0, first.stderr); assert.equal(second.status, 0, second.stderr);
    const cold = JSON.parse(first.stdout), warm = JSON.parse(second.stdout);
    assert.equal(cold.cached, false); assert.equal(warm.cached, true);
    assert.equal(cold.fetchedAt, warm.fetchedAt);
    assert.match(warm.evidence, /°F/);
    assert.match(first.stderr, /HTTP_CALLS=2/); assert.match(second.stderr, /HTTP_CALLS=0/);
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
