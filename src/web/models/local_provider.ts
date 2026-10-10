/**
 * Local OpenAI-Compatible LLM Provider: Connects to local llama.cpp server,
 * Ollama, vLLM, or LM Studio endpoints with schema constraints and timeouts.
 */

import type { LLMProvider, GenerationRequest, GenerationResponse } from '../types';

export interface LocalServerOptions {
  idSlot?: number;
  deadlineMs?: number;
  baseUrl?: string;
  modelName?: string;
  timeoutMs?: number;
  apiKey?: string;
  maxInputTokens?: number;
}

export class LocalOpenAICompatibleProvider implements LLMProvider {
  private baseUrl: string;
  private modelName: string;
  private timeoutMs: number;
  private apiKey?: string;
  private maxInputTokens: number;
  private idSlot?: number;
  private pending: Promise<unknown> = Promise.resolve();
  private deadlineMs: number;

  constructor(options: LocalServerOptions = {}) {
    this.baseUrl = options.baseUrl ?? 'http://127.0.0.1:8080/v1';
    this.modelName = options.modelName ?? 'local-model';
    this.timeoutMs = options.timeoutMs ?? 30000;
    this.apiKey = options.apiKey;
    this.maxInputTokens = options.maxInputTokens ?? 6000;
    this.idSlot = options.idSlot;
    this.deadlineMs = options.deadlineMs ?? Infinity;
    const endpoint = new URL(this.baseUrl);
    const host = endpoint.hostname.replace(/\.$/, '');
    const loopback = ['localhost', '[::1]'].includes(host) || /^127(?:\.\d{1,3}){3}$/.test(host);
    if (!loopback || !['http:','https:'].includes(endpoint.protocol) || endpoint.username || endpoint.password) throw new Error('Local model endpoint must be loopback HTTP(S)');
  }

  /** Prefer the loaded llama.cpp tokenizer; byte count is a conservative fallback. */
  async countTokens(text: string): Promise<number> {
    try {
      const response = await fetch(new URL('/tokenize', this.baseUrl), {method:'POST',
        headers:{'Content-Type':'application/json', ...(this.apiKey ? {Authorization:`Bearer ${this.apiKey}`} : {})},
        body:JSON.stringify({content:text,add_special:true}),signal:AbortSignal.timeout(2000),redirect:'error'});
      if (response.ok) {
        const data = await response.json();
        if (Array.isArray(data.tokens)) return data.tokens.length;
      }
    } catch { /* Runtime does not expose a tokenizer. Fail conservatively below. */ }
    return new TextEncoder().encode(text).length;
  }

  async generate(request: GenerationRequest): Promise<GenerationResponse> {
    const result = this.pending.then(() => this.generateOnce(request));
    this.pending = result.catch(() => {});
    return result;
  }

  private async generateOnce(request: GenerationRequest): Promise<GenerationResponse> {
    if (Date.now() >= this.deadlineMs) throw new Error('Research deadline exceeded');
    const url = `${this.baseUrl.replace(/\/+$/, '')}/chat/completions`;

    const messages = [
      { role: 'system', content: request.systemPrompt },
      { role: 'user', content: request.userPrompt },
    ];

    const body: Record<string, unknown> = {
      model: this.modelName,
      messages,
      temperature: request.temperature ?? 0.2,
      max_tokens: request.maxTokens ?? 1024,
    };
    if (request.disableThinking) body.chat_template_kwargs = { enable_thinking: false };
    if (this.idSlot !== undefined) body.id_slot = this.idSlot;

    if (request.responseSchema) {
      body.response_format = {
        type: 'json_schema',
        json_schema: { name: 'research', schema: request.responseSchema },
      };
    }

    if (request.stopSequences && request.stopSequences.length > 0) {
      body.stop = request.stopSequences;
    }

    let inputTokens: number | undefined;
    try {
      const response = await fetch(`${url}/input_tokens`, { method: 'POST',
        headers: { 'Content-Type': 'application/json', ...(this.apiKey ? { Authorization: `Bearer ${this.apiKey}` } : {}) },
        body: JSON.stringify(body), signal: AbortSignal.timeout(2000), redirect: 'error' });
      if (response.ok) {
        const data = await response.json();
        if (Number.isInteger(data.input_tokens) && data.input_tokens >= 0) inputTokens = data.input_tokens;
      }
    } catch { /* Compatible providers may only expose /tokenize. */ }
    inputTokens ??= await this.countTokens(request.systemPrompt + '\n' + request.userPrompt) + 64;
    if (inputTokens > this.maxInputTokens) throw new Error('Model input exceeds the research token budget');

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), Math.min(this.timeoutMs, this.deadlineMs - Date.now()));

    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };
    if (this.apiKey) {
      headers['Authorization'] = `Bearer ${this.apiKey}`;
    }

    try {
      const res = await fetch(url, {
        method: 'POST',
        headers,
        body: JSON.stringify(body),
        signal: controller.signal,
        redirect: 'error',
      });

      if (!res.ok) {
        throw new Error(`Local LLM server returned HTTP ${res.status}: ${res.statusText}`);
      }

      const data = await res.json();
      clearTimeout(timer);
      const choice = data.choices?.[0];
      const text = choice?.message?.content || '';

      return {
        text,
        tokensUsed: data.usage?.total_tokens,
        finishReason: choice?.finish_reason,
      };
    } catch (err: any) {
      clearTimeout(timer);
      if (err.name === 'AbortError') {
        throw new Error(`Local LLM generation timed out after ${this.timeoutMs}ms`);
      }
      throw err;
    }
  }
}
