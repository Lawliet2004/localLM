const DIAGNOSTIC_BODY_LIMIT = 4000;

export class FetchFailure extends Error {
  readonly status?: number;
  readonly diagnosticBody?: string;

  constructor(message: string, status?: number, diagnosticBody?: string) {
    super(message);
    this.name = 'FetchFailure';
    if (Number.isInteger(status)) this.status = status;
    if (diagnosticBody) this.diagnosticBody = diagnosticBody.slice(0, DIAGNOSTIC_BODY_LIMIT);
  }
}

export function shapeFailedFetch(url: string, error: unknown) {
  const failure = error instanceof FetchFailure ? error : undefined;
  const diagnosticBody = failure?.diagnosticBody?.slice(0, DIAGNOSTIC_BODY_LIMIT);
  return {
    url,
    isError: true,
    failureKind: 'fetch_failed',
    ...(failure?.status !== undefined ? { status: failure.status } : {}),
    ...(diagnosticBody ? { diagnosticBody } : {}),
    message: `${error instanceof Error ? error.message : String(error)}${diagnosticBody ? ' Diagnostic body is not research evidence.' : ''}`,
  };
}
