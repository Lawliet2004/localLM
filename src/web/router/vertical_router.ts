/**
 * Vertical Router: Determines whether a request belongs to a specialized structured vertical
 * (weather, currency, time), static internal knowledge (NONE), or general web search.
 */

import type { VerticalType } from '../types';

export function routeVertical(query: string): VerticalType {
  const q = query.toLowerCase().trim();

  // 1. Weather
  if (
    /\b(weather|temperature|forecast|rain|snow|humidity|wind speed|weather report)\b/i.test(q)
  ) {
    return 'WEATHER';
  }

  // 2. Currency
  if (
    /\b(exchange rate|currency converter|usd to|eur to|inr to|gbp to|jpy to|aud to|cad to)\b/i.test(q) ||
    /\bconvert\s+\d+(\.\d+)?\s+[a-z]{3}\s+to\s+[a-z]{3}\b/i.test(q)
  ) {
    return 'CURRENCY';
  }

  // 3. Time
  if (
    /\b(current time in|what time is it in|timezone of|local time in)\b/i.test(q)
  ) {
    return 'TIME';
  }

  // 4. Static / Conceptual knowledge (no external search needed)
  if (
    /^(what (is|does)|how does|explain|define)\s+(polymorphism|recursion|quicksort|binary search|photosynthesis|entropy|mitosis|encapsulation|gravity)\b/i.test(q) &&
    !/\b(latest|recent|new|today|2026|current)\b/i.test(q)
  ) {
    return 'NONE';
  }

  // 5. Documentation
  if (/\b(docs|documentation|api reference|sdk documentation|guide for)\b/i.test(q)) {
    return 'DOCUMENTATION';
  }

  // 6. News
  if (/\b(breaking news|headlines|what happened (today|yesterday|with))\b/i.test(q)) {
    return 'NEWS';
  }

  // 7. Academic / Benchmarks
  if (/\b(research paper|arxiv|benchmark comparison|eval results|swe-bench)\b/i.test(q)) {
    return 'ACADEMIC';
  }

  // Default to general web
  return 'GENERAL_WEB';
}
