/** Decorative activity only; the owning status or disclosure supplies its name. */
export function ActivityIndicator({ active = true }: { active?: boolean }) {
  return <span className="activity-indicator" data-active={active} aria-hidden="true">
    <span /><span /><span />
  </span>;
}
