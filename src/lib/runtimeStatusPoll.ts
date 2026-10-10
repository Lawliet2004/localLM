import { useEffect } from 'react';
import { api, nativeAvailable } from './api';
import type { RuntimeStatus } from './types';

// While the local runtime is loading or running, poll its status so an
// externally exited llama-server is noticed. Stops for stopped/error phases.
export function useRuntimeStatusPoll(
  phase: RuntimeStatus['phase'],
  apply: (runtime: RuntimeStatus) => void,
) {
  useEffect(() => {
    if (!nativeAvailable || (phase !== 'loading' && phase !== 'ready')) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const runtime = await api.runtimeStatus();
        if (!disposed) apply(runtime);
      } catch { /* Keep the last known status on a transient failure. */ }
      if (!disposed) timer = setTimeout(() => void poll(), 3000);
    }
    timer = setTimeout(() => void poll(), 3000);
    return () => { disposed = true; clearTimeout(timer); };
  }, [phase, apply]);
}
