import type { EmbeddingProvider, EvidenceChunk, Reranker } from '../types';
import type { StorageAdapter } from '../cache/sqlite';

export interface LocalRetrievalOptions { baseUrl: string; model: string }
async function post(options: LocalRetrievalOptions, path: string, body: Record<string, unknown>): Promise<any> {
  const url = new URL(path, options.baseUrl);
  if (!['localhost','127.0.0.1','[::1]'].includes(url.hostname) || url.protocol !== 'http:') throw new Error('Retrieval model endpoints must be local HTTP servers');
  const response = await fetch(url, {method:'POST', headers:{'Content-Type':'application/json'},
    body:JSON.stringify({...body, model: options.model}), signal:AbortSignal.timeout(20000), redirect:'error'});
  if (!response.ok) throw new Error(`Local retrieval model returned HTTP ${response.status}`);
  return response.json();
}

/** Uses a trained local embedding model, never a hosted embedding API. */
export class LocalEmbeddingProvider implements EmbeddingProvider {
  constructor(private options: LocalRetrievalOptions, private storage?: StorageAdapter) {}
  async embedQuery(text: string): Promise<number[]> { return (await this.embedDocuments([text]))[0]; }
  async embedDocuments(texts: string[]): Promise<number[][]> {
    const output: number[][] = [];
    for (const text of texts) {
      const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
      const key = `${this.options.baseUrl}:${this.options.model}:${Array.from(new Uint8Array(digest)).map(b => b.toString(16).padStart(2,'0')).join('')}`;
      let vector = await this.storage?.get<number[]>('embedding_cache', key);
      if (!vector) {
        const data = await post(this.options, '/v1/embeddings', {input: text});
        vector = data.data?.[0]?.embedding;
        if (!Array.isArray(vector) || !vector.length || !vector.every(Number.isFinite)) throw new Error('Invalid local embedding response');
        await this.storage?.set('embedding_cache', key, vector, 2592000);
      }
      output.push(vector);
    }
    return output;
  }
}

/** llama.cpp /reranking endpoint backed by a separately loaded cross-encoder. */
export class LocalModelReranker implements Reranker {
  constructor(private options: LocalRetrievalOptions) {}
  async rerank(query: string, chunks: EvidenceChunk[], limit: number): Promise<EvidenceChunk[]> {
    if (!chunks.length) return [];
    const data = await post(this.options, '/reranking', {query, documents:chunks.map(c => c.text), top_n:limit});
    const rows = data.results ?? data.data;
    if (!Array.isArray(rows)) throw new Error('Invalid local reranker response');
    const seen = new Set<number>();
    return rows.filter(r => Number.isInteger(r.index) && r.index >= 0 && r.index < chunks.length && Number.isFinite(r.relevance_score))
      .sort((a,b) => b.relevance_score - a.relevance_score).filter(r => {
        if (seen.has(r.index)) return false;
        seen.add(r.index); return true;
      }).slice(0,limit).map(r => ({...chunks[r.index],rerankScore:r.relevance_score}));
  }
}
