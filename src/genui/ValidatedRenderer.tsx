import { useMemo, type ReactNode } from 'react';
import { Renderer, type RendererProps } from '@openuidev/react-lang';
import { locallmLibrary } from './library';
import { validateProgram } from './validation';
import { progressiveProgram } from './progressive';
import { VisualErrorBoundary } from './VisualErrorBoundary';

type Props = Omit<RendererProps<typeof locallmLibrary>, 'library' | 'toolProvider' | 'response'> & {
  response: string;
  fallback?: ReactNode;
  onRenderError?: (error: Error) => void;
};

/** Shared by chat and canvas. No tool provider, remote resources or host API
 * are injected into the official renderer. Boundary identity stays stable
 * across token updates so component-local state survives valid streaming.
 */
export function ValidatedRenderer({ response, isStreaming = false, fallback, onRenderError, ...props }: Props) {
  const committed = useMemo(() => isStreaming ? progressiveProgram(response) : response, [response, isStreaming]);
  const validation = useMemo(() => validateProgram(committed ?? "", isStreaming), [committed, isStreaming]);
  if (!validation.valid) {
    return <p className="genui-block-note" role="status">
      {isStreaming ? 'Building visual answer…' : `Couldn’t render this UI. ${validation.diagnostic} Its source is still available.`}
    </p>;
  }
  return <VisualErrorBoundary resetKey={response} fallback={fallback} onError={onRenderError}>
    <Renderer {...props} response={committed!} library={locallmLibrary}
      isStreaming={isStreaming} toolProvider={null} publishObservability={false} />
  </VisualErrorBoundary>;
}
