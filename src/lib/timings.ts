import type { RoundTimings } from './types';

const count = (value: number) => Math.round(value).toLocaleString();

/** One-line readout of measured runtime timings. Unreported values are left out rather than estimated. */
export function formatTimings(timings: RoundTimings): string | null {
  const parts: string[] = [];
  const cached = timings.cacheN ?? null;
  const evaluated = timings.promptN ?? null;
  if (evaluated != null || cached != null) {
    const total = (evaluated ?? 0) + (cached ?? 0);
    const ratio = timings.cacheHitRatio;
    parts.push(cached != null
      ? `${count(total)} prompt tokens (${count(cached)} from cache${ratio != null ? `, ${Math.round(ratio * 100)}%` : ''})`
      : `${count(total)} prompt tokens evaluated`);
  }
  if (timings.promptPerSecond != null) parts.push(`prefill ${count(timings.promptPerSecond)} tok/s`);
  if (timings.predictedN != null) parts.push(`${count(timings.predictedN)} generated`);
  if (timings.predictedPerSecond != null) parts.push(`decode ${timings.predictedPerSecond.toFixed(1)} tok/s`);
  return parts.length ? `Last round: ${parts.join(' · ')}` : null;
}
