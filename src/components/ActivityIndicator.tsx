/** Decorative activity only; the owning status or disclosure supplies its name.
 * One rotating arc while work is live, a still mark once it is done. */
export function ActivityIndicator({ active = true }: { active?: boolean }) {
  return <span className="activity-indicator" data-active={active} aria-hidden="true">
    <span />
  </span>;
}
