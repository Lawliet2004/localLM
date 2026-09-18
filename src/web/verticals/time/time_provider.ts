/**
 * Time Vertical Provider: Zero-cost world clock and timezone lookup.
 */

import type { VerticalProvider } from '../../types';

export interface TimeReport {
  location: string;
  timezone: string;
  timeFormatted: string;
  dateFormatted: string;
  iso: string;
  utcOffset: string;
}

const CITY_TIMEZONES: Record<string, string> = {
  tokyo: 'Asia/Tokyo',
  japan: 'Asia/Tokyo',
  london: 'Europe/London',
  uk: 'Europe/London',
  'new york': 'America/New_York',
  nyc: 'America/New_York',
  paris: 'Europe/Paris',
  berlin: 'Europe/Berlin',
  kolkata: 'Asia/Kolkata',
  india: 'Asia/Kolkata',
  ranaghat: 'Asia/Kolkata',
  delhi: 'Asia/Kolkata',
  sydney: 'Australia/Sydney',
  dubai: 'Asia/Dubai',
  singapore: 'Asia/Singapore',
  utc: 'UTC',
  gmt: 'UTC',
};

export class TimeProvider implements VerticalProvider<string, TimeReport> {
  resolveTimezone(query: string): { location: string; timezone: string } {
    const q = query.toLowerCase();
    for (const [city, tz] of Object.entries(CITY_TIMEZONES)) {
      if (q.includes(city)) {
        return { location: city.toUpperCase(), timezone: tz };
      }
    }
    return { location: 'UTC', timezone: 'UTC' };
  }

  async execute(query: string): Promise<TimeReport> {
    const { location, timezone } = this.resolveTimezone(query);
    const now = new Date();

    const timeFormatter = new Intl.DateTimeFormat('en-US', {
      timeZone: timezone,
      hour: 'numeric',
      minute: 'numeric',
      second: 'numeric',
      hour12: true,
    });

    const dateFormatter = new Intl.DateTimeFormat('en-US', {
      timeZone: timezone,
      weekday: 'long',
      year: 'numeric',
      month: 'long',
      day: 'numeric',
    });

    const timeFormatted = timeFormatter.format(now);
    const dateFormatted = dateFormatter.format(now);

    return {
      location,
      timezone,
      timeFormatted,
      dateFormatted,
      iso: now.toISOString(),
      utcOffset: timezone,
    };
  }

  formatReportAsEvidence(report: TimeReport): string {
    return `Current time in ${report.location} (${report.timezone}): ${report.timeFormatted}, ${report.dateFormatted}`;
  }
}
