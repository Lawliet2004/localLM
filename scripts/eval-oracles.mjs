const EARTH_RADIUS_KM = 6378.137;
const LOW_ALTITUDE_KM = 300;
const GEO_ALTITUDE_KM = 35_786;
const EARTH_MU_KM3_S2 = 398_600.4418;

export const HOHMANN_TRANSFER_SECONDS = Math.PI * Math.sqrt(
  (((EARTH_RADIUS_KM + LOW_ALTITUDE_KM) + (EARTH_RADIUS_KM + GEO_ALTITUDE_KM)) / 2) ** 3
    / EARTH_MU_KM3_S2,
);

export function checkHohmannTransferTime(messages) {
  const answer = [...messages].reverse().find(message => message?.role === 'assistant' && message?.status === 'complete')?.content ?? '';
  const relevant = answer.split(/\r?\n/).filter(line => /transfer\s*time|\bT_t\b/i.test(line));
  const observedSeconds = [];
  const time = /\b(\d[\d,]*(?:\.\d+)?(?:e[+-]?\d+)?)\s*(seconds?|secs?|s|hours?|hrs?|h)\b/gi;
  const superscriptDigits = { '⁰': '0', '¹': '1', '²': '2', '³': '3', '⁴': '4', '⁵': '5', '⁶': '6', '⁷': '7', '⁸': '8', '⁹': '9' };
  for (const line of relevant) {
    const normalized = line.replace(/\s*[×x*]\s*10\s*\^?\s*([⁰¹²³⁴⁵⁶⁷⁸⁹]+|\d+)/g,
      (_, exponent) => `e${[...exponent].map(digit => superscriptDigits[digit] ?? digit).join('')}`);
    for (const match of normalized.matchAll(time)) {
      const value = Number(match[1].replaceAll(',', ''));
      if (Number.isFinite(value)) observedSeconds.push(/^h/i.test(match[2]) ? value * 3600 : value);
    }
  }
  const toleranceSeconds = HOHMANN_TRANSFER_SECONDS * 0.02;
  return {
    verdict: observedSeconds.length === 0 ? 'unparsed'
      : observedSeconds.some(seconds => Math.abs(seconds - HOHMANN_TRANSFER_SECONDS) <= toleranceSeconds) ? 'pass' : 'fail',
    expectedSeconds: Number(HOHMANN_TRANSFER_SECONDS.toFixed(3)),
    observedSeconds,
    toleranceSeconds: Number(toleranceSeconds.toFixed(3)),
  };
}

export function evaluateKnownAnswer(testId, messages) {
  return testId === 4 ? { hohmannTransferTime: checkHohmannTransferTime(messages) } : {};
}
