import type { KeyboardEvent } from 'react';

/**
 * Arrow-key movement for a tablist or radiogroup: Left/Up and Right/Down wrap
 * through the options, select the target via `select`, then move focus to the
 * matching child once React has committed. `currentTarget` is captured because
 * React nulls it before the animation frame runs; the group's children are
 * assumed to be in `values` order.
 */
export function moveWithinGroup<T>(event: KeyboardEvent<HTMLElement>, values: readonly T[], current: T, select: (value: T) => void): void {
  if (event.key !== 'ArrowRight' && event.key !== 'ArrowDown' && event.key !== 'ArrowLeft' && event.key !== 'ArrowUp') return;
  event.preventDefault();
  const group = event.currentTarget;
  const index = values.indexOf(current);
  if (index < 0) return;
  const next = event.key === 'ArrowRight' || event.key === 'ArrowDown'
    ? (index + 1) % values.length : (index - 1 + values.length) % values.length;
  select(values[next]);
  requestAnimationFrame(() => (group.children[next] as HTMLElement | undefined)?.focus());
}
