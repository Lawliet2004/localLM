import { useEffect, useState } from 'react';
import { api, errorMessage } from '../lib/api';
import { WeatherResult, asWeatherReport } from './WeatherResult';
import { ExecutionArtifacts } from './ExecutionArtifacts';

/** Full result stays in UI storage, outside the model's compact tool context. */
export function ToolResultExtras({ name, result, conversationId }: { name: string; result: Record<string, unknown> | null; conversationId: string }) {
  const [full, setFull] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState('');
  const relevant = /^(?:get_weather|weather|run_code|local_run_code|execute_command|local_execute_command)$/.test(name);
  const id = typeof result?._artifactId === 'string' ? result._artifactId : null;
  useEffect(() => {
    setFull(null); setError('');
    if (!relevant || !id) return;
    let disposed = false;
    api.getArtifact(id).then(artifact => {
      if (disposed) return;
      if (!artifact || artifact.conversationId !== conversationId || artifact.mimeType !== 'application/json') throw new Error('Full tool result is unavailable in this conversation.');
      const value: unknown = JSON.parse(artifact.content);
      if (value && typeof value === 'object' && !Array.isArray(value)) setFull(value as Record<string, unknown>);
    }).catch(e => { if (!disposed) setError(errorMessage(e)); });
    return () => { disposed = true; };
  }, [id, relevant, conversationId]);
  if (!relevant) return null;
  const value = full || result;
  const weather = /^(?:get_weather|weather)$/.test(name) ? asWeatherReport(value) : null;
  return <>{weather && <WeatherResult report={weather} />}{value && /^(?:run_code|local_run_code|execute_command|local_execute_command)$/.test(name) && <ExecutionArtifacts result={value} />}{error && <p role="alert">{error}</p>}</>;
}
