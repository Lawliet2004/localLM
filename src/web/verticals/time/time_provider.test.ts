import { expect, it } from 'vitest';
import { TimeProvider } from './time_provider';

it('resolves whole city names and explicit IANA timezones without inventing UTC', async () => {
  const provider = new TimeProvider();
  expect(provider.resolveTimezone('current time in New York')).toEqual({ location: 'NEW YORK', timezone: 'America/New_York' });
  expect(provider.resolveTimezone('current time in Europe/Rome').timezone).toBe('Europe/Rome');
  expect(() => provider.resolveTimezone('current time in Phuket')).toThrow(/Could not resolve/);
  expect(() => provider.resolveTimezone('current time in Atlantis')).toThrow(/Could not resolve/);
  expect((await provider.execute('current time in Kolkata')).utcOffset).toBe('+05:30');
  expect((await provider.execute('current time in UTC')).utcOffset).toBe('+00:00');
});
