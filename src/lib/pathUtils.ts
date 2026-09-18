/**
 * Compare paths as the local operating system does.
 *
 * Windows may return a canonical path with the `\\?\\` prefix while the
 * settings store keeps the user-facing `C:\\...` spelling. Those two strings
 * point at the same file and must not be treated as two different models.
 */
export function comparablePath(value: string | null | undefined): string {
  const original = (value ?? '').trim();
  if (!original) return '';
  const windowsPath = original.startsWith('\\\\?\\') || original.startsWith('\\\\') || /^[A-Za-z]:[\\/]/.test(original);
  const withoutDevicePrefix = original.replace(/^\\\\\?\\/, '');
  const normalized = withoutDevicePrefix.replace(/[\\/]+$/, '').replace(/\\/g, '/');
  return windowsPath ? normalized.toLowerCase() : normalized;
}

export function samePath(left: string | null | undefined, right: string | null | undefined): boolean {
  const a = comparablePath(left);
  const b = comparablePath(right);
  return Boolean(a && b && a === b);
}
