import { createRoot } from 'react-dom/client';
import { PermissionSelector } from '../src/components/ToolControls';

/* Screenshot fixture: nativeAvailable is false in a plain browser, which
   disables the trigger, so mount the selector with it forced on. */
export function mountPermissionPreview() {
  const host = document.createElement('div');
  host.style.cssText = 'position:fixed;left:32px;bottom:170px;z-index:10000';
  document.body.append(host);
  createRoot(host).render(<PermissionSelector accessMode="fullAccess" nativeAvailable onAccessModeChange={() => {}} />);
}
