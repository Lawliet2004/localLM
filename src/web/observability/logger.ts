/**
 * Structured Logger: Emits structured diagnostic event logs across pipeline stages.
 */

export type PipelineStage =
  | 'request_received'
  | 'request_normalized'
  | 'route_selected'
  | 'queries_generated'
  | 'search_completed'
  | 'search_pagination'
  | 'search_suggestion'
  | 'results_deduplicated'
  | 'pages_selected'
  | 'fetch_completed'
  | 'extraction_completed'
  | 'chunks_created'
  | 'retrieval_completed'
  | 'rerank_completed'
  | 'evidence_created'
  | 'context_built'
  | 'answer_generated'
  | 'verification_completed'
  | 'retry_search_initiated'
  | 'search_fallback';

export interface LogEvent {
  timestamp: string;
  stage: PipelineStage;
  data: Record<string, unknown>;
}

export class PipelineLogger {
  private events: LogEvent[] = [];
  private enabled: boolean;

  constructor(enabled: boolean = false) {
    this.enabled = enabled;
  }

  log(stage: PipelineStage, data: Record<string, unknown> = {}) {
    const event: LogEvent = {
      timestamp: new Date().toISOString(),
      stage,
      data,
    };
    this.events.push(event);
    if (this.events.length > 200) this.events.shift();

    if (this.enabled) {
      console.log(`[WebEngine:${stage}]`, JSON.stringify(data));
    }
  }

  getEvents(): LogEvent[] {
    return this.events;
  }
}
