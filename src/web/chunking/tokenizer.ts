/**
 * Token Counter: Estimates token counts accurately for LLM context planning.
 */

import type { TokenCounter } from '../types';

export class ApproximateTokenCounter implements TokenCounter {
  count(text: string): number {
    if (!text) return 0;
    // Hybrid word & punctuation tokenizer:
    // Models typically produce ~1 token per 3.8-4 characters for English prose,
    // or ~1.3 tokens per whitespace-separated word.
    const words = text.trim().split(/\s+/).filter(Boolean);
    const charEstimate = Math.ceil(text.length / 3.8);
    const wordEstimate = Math.ceil(words.length * 1.33);

    // Planning estimate only. The local provider checks its tokenizer before inference.
    return Math.max(charEstimate, wordEstimate);
  }
}

export const defaultTokenCounter = new ApproximateTokenCounter();
