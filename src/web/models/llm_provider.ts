/**
 * LLM Provider: Standardized interface and mock implementations for local model inference.
 */

import type { LLMProvider, GenerationRequest, GenerationResponse } from '../types';

export type { LLMProvider, GenerationRequest, GenerationResponse };

export class MockLLMProvider implements LLMProvider {
  constructor(private cannedResponses: Record<string, string> = {}) {}

  async generate(request: GenerationRequest): Promise<GenerationResponse> {
    const prompt = (request.userPrompt + ' ' + request.systemPrompt).toLowerCase();

    for (const [trigger, reply] of Object.entries(this.cannedResponses)) {
      if (prompt.includes(trigger.toLowerCase())) {
        return { text: reply };
      }
    }

    // Default mock response
    return {
      text: 'Mock grounded response with evidence [S1].',
    };
  }
}
