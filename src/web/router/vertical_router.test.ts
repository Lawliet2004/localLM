import { describe, it, expect } from 'vitest';
import { routeVertical } from './vertical_router';
import { routeRequest } from './intent_router';

describe('Vertical Router: weather intent', () => {
  it('does not route research queries merely containing "temperature" to weather', () => {
    expect(routeVertical('room-temperature superconductivity research')).toBe('GENERAL_WEB');
    expect(routeVertical('What is the latest on room-temperature superconductivity?')).not.toBe('WEATHER');
    expect(routeRequest('room-temperature superconductivity research').requiresWebSearch).toBe(true);
  });

  it('still routes genuine weather queries to the weather vertical', () => {
    expect(routeVertical('temperature in Paris tomorrow weather')).toBe('WEATHER');
    expect(routeVertical('weather forecast for London')).toBe('WEATHER');
    expect(routeVertical('will it rain in New York today?')).toBe('WEATHER');
    expect(routeVertical('humidity and wind speed in Kolkata right now')).toBe('WEATHER');
  });
});
