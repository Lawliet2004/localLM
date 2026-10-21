import type { AccessMode } from '../lib/types';

export const accessModes: { value: AccessMode; label: string; name: string }[] = [
  { value: 'ask', label: 'Ask', name: 'Ask for approval' },
  { value: 'autoApprove', label: 'Auto-approve', name: 'Auto-approve reads' },
  { value: 'fullAccess', label: 'Full access', name: 'Full access' },
];

export const approvalNotes: Record<AccessMode, string> = {
  ask: 'Every tool action needs your approval.',
  autoApprove: 'Workspace reads run automatically. Other actions ask.',
  fullAccess: 'Enabled tools run automatically unless their settings require approval.',
};

export function ApprovalSegmented({ value, onChange, disabled, name, labelledBy }: {
  value: AccessMode;
  onChange: (mode: AccessMode) => void;
  disabled?: boolean;
  name: string;
  labelledBy: string;
}) {
  return (
    <div className={`segmented${disabled ? ' is-disabled' : ''}`} role="radiogroup" aria-labelledby={labelledBy}>
      {accessModes.map(({ value: mode, label, name: accessibleName }) => <label className="segment" key={mode}>
        <input type="radio" name={name} value={mode} aria-label={accessibleName} checked={value === mode} disabled={disabled} onChange={() => onChange(mode)} />
        <span>{label}</span>
      </label>)}
    </div>
  );
}
