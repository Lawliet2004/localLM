/**
 * Core type definitions for the zero-cost, local-first web search,
 * retrieval, grounding, citation, and verification engine.
 */

export type FreshnessWindow = 'day' | 'week' | 'month' | 'year' | 'any' | 'realtime';

export type VerticalType =
  | 'NONE'
  | 'GENERAL_WEB'
  | 'NEWS'
  | 'WEATHER'
  | 'CURRENCY'
  | 'TIME'
  | 'SPORTS'
  | 'MARKET'
  | 'DOCUMENTATION'
  | 'CODE'
  | 'ACADEMIC'
  | 'PRODUCT'
  | 'LOCAL';

export type SourceType =
  | 'official_documentation'
  | 'academic_paper'
  | 'primary_source'
  | 'news_organization'
  | 'specialist_publication'
  | 'community_forum'
  | 'social_media'
  | 'unknown';

export type ResearchMode = 'fast' | 'normal' | 'deep';

export type ClaimStatus =
  | 'SUPPORTED'
  | 'PARTIALLY_SUPPORTED'
  | 'UNSUPPORTED'
  | 'CONFLICTING'
  | 'NOT_EXTERNALLY_VERIFIABLE';

export interface SearchRequest {
  query: string;
  category?: string;
  language?: string;
  freshness?: FreshnessWindow;
  maxResults?: number;
  /** SearXNG result page (1-5); enables selective pagination past page one. */
  page?: number;
}

export interface SearchResult {
  id: string;
  queryId: string;
  title: string;
  url: string;
  canonicalUrl?: string;
  snippet?: string;
  domain: string;
  publishedAt?: string;
  engine?: string;
  rank: number;
  score?: number;
  metadata?: Record<string, unknown>;
}

export interface RetrievedDocument {
  id: string;
  url: string;
  canonicalUrl?: string;
  domain: string;
  title: string;
  author?: string;
  publishedAt?: string;
  text: string;
  /** PDF page texts; index i is page i+1. Enables page references. */
  pages?: string[];
  /** Heading outline for section navigation. */
  headings?: string[];
  /** Followable outbound links (bounded). */
  links?: Array<{ text: string; href: string }>;
  sourceType?: SourceType;
  authorityScore?: number;
  retrievedAt: string;
  searchResultIds: string[];
  contentHash?: string;
  metadata?: Record<string, unknown>;
}

export interface EvidenceChunk {
  id: string;
  documentId: string;
  url: string;
  title: string;
  headingPath?: string[];
  text: string;
  tokenCount: number;
  startOffset?: number;
  endOffset?: number;
  publishedAt?: string;
  lexicalScore?: number;
  semanticScore?: number;
  rerankScore?: number;
}

export interface ExtractedFact {
  statement: string;
  confidence: number;
  sourceId: string;
  chunkId: string;
}

export interface EvidenceClaim {
  id: string;
  claim: string;
  supportingSources: string[];
  conflictingSources?: string[];
  status: 'supported' | 'conflicting' | 'unsupported';
  confidence: number;
  variants?: string[];
}

export interface SourceMeta {
  id: string; // e.g. "S1"
  title: string;
  url: string;
  domain: string;
  publishedAt?: string;
  author?: string;
  snippet?: string;
  sourceType?: SourceType;
  authorityScore?: number;
}

export type SourceRegistry = Record<string, SourceMeta>;

export interface RouteResult {
  requiresExternalData: boolean;
  requiresWebSearch: boolean;
  vertical: VerticalType;
  freshness: FreshnessWindow;
  confidence: number;
  complexity?: 'simple' | 'medium' | 'complex';
  reasoning?: string;
}

export interface PlannedQuery {
  query: string;
  purpose: string;
  freshness: FreshnessWindow;
  /** SearXNG result page; follow-up pagination only, defaults to 1. */
  page?: number;
}

export interface QueryPlan {
  queries: PlannedQuery[];
  requirements?: Array<{ id: string; description: string }>;
}

export interface TokenStats {
  systemBudget: number;
  questionBudget: number;
  evidenceBudget: number;
  safetyMargin: number;
  totalInputBudget: number;
  estimatedEvidenceTokens: number;
  finalPromptTokens: number;
}

export interface VerifiedClaim {
  claim: string;
  status: ClaimStatus;
  sources: string[];
  explanation?: string;
}

export interface VerificationReport {
  claims: VerifiedClaim[];
  allSupported: boolean;
  supportedCount: number;
  unsupportedCount: number;
  conflictingCount: number;
}

export interface ResearchTrace {
  queriesGenerated: number;
  searchResults: number;
  uniqueResults: number;
  pagesSelected: number;
  pagesFetched: number;
  fetchFailures: number;
  /** Per-query failures: healthy results are kept, failures stay visible. */
  queryFailures?: Array<{ query: string; error: string }>;
  searchFailureCount?: number;
  searxngDiagnostics?: Record<string, unknown>;
  paginationPages?: number[];
  extractedTokens: number;
  chunksCreated: number;
  chunksAfterRetrieval: number;
  chunksAfterRerank: number;
  evidenceClaims: number;
  finalEvidenceTokens: number;
  finalPromptTokens: number;
  verifiedClaims: number;
  unsupportedClaims: number;
  conflictingClaims: number;
  providerUsed: string;
  fallbackUsed: boolean;
  fallbackReason: string;
  latencies: Record<string, number>;
  totalLatencyMs: number;
}

export interface ResearchSession {
  coverage?: Array<{entity: string; dimension: string; covered: boolean}>;
  requirements?: Array<{ id: string; text: string; status: 'answered' | 'unresolved' | 'blocked'; reason?: string }>;
  researchState?: {
    openQuestions: string[];
    contradictions: Array<{ claims: string[]; sources: string[] }>;
    rejected: Array<{ candidate: string; reason: string }>;
    completedActions: string[];
    pendingActions: string[];
  };
  limitations?: string[];
  id: string;
  question: string;
  normalizedQuery: string;
  route: RouteResult;
  queries: PlannedQuery[];
  results: SearchResult[];
  documents: RetrievedDocument[];
  chunks: EvidenceChunk[];
  retrievedChunks: EvidenceChunk[];
  rerankedChunks: EvidenceChunk[];
  evidence: EvidenceClaim[];
  sources: SourceRegistry;
  tokenUsage: TokenStats;
  answer?: string;
  verification?: VerificationReport;
  startedAt: string;
  completedAt?: string;
  trace: ResearchTrace;
}

export interface GenerationRequest {
  systemPrompt: string;
  userPrompt: string;
  temperature?: number;
  maxTokens?: number;
  stopSequences?: string[];
  responseSchema?: Record<string, unknown>;
}

export interface GenerationResponse {
  text: string;
  tokensUsed?: number;
  finishReason?: string;
}

export interface LLMProvider {
  countTokens?(text: string): Promise<number>;
  generate(request: GenerationRequest): Promise<GenerationResponse>;
}

export interface EmbeddingProvider {
  embedQuery(text: string): Promise<number[]>;
  embedDocuments(texts: string[]): Promise<number[][]>;
}

export interface TokenCounter {
  count(text: string): number;
}

export interface Reranker {
  rerank(query: string, chunks: EvidenceChunk[], limit: number): Promise<EvidenceChunk[]>;
}

export interface SearchProvider {
  search(request: SearchRequest): Promise<SearchResult[]>;
}

export interface VerticalProvider<TRequest, TResponse> {
  execute(request: TRequest): Promise<TResponse>;
}
