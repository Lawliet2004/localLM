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

/** SearXNG direct answer from an instant-answer plugin. */
export interface SearchAnswer {
  answer: string;
  url?: string;
}

/** SearXNG infobox (Wikipedia/Wikidata side panel content). */
export interface SearchInfobox {
  title: string;
  content: string;
  url?: string;
}

/**
 * Structured SearXNG response fields beyond organic results: instant answers,
 * infoboxes, spelling corrections and related-search suggestions.
 */
export interface SearchMeta {
  answers: SearchAnswer[];
  infoboxes: SearchInfobox[];
  corrections: string[];
  suggestions: string[];
}

export function emptySearchMeta(): SearchMeta {
  return { answers: [], infoboxes: [], corrections: [], suggestions: [] };
}

export function hasSearchMeta(meta: SearchMeta): boolean {
  return meta.answers.length > 0 || meta.infoboxes.length > 0
    || meta.corrections.length > 0 || meta.suggestions.length > 0;
}

/** Merge SearXNG meta from concurrent/paginated queries; first-seen wins. */
export function mergeSearchMeta(base: SearchMeta, extra: SearchMeta): SearchMeta {
  const answers = [...base.answers];
  for (const a of extra.answers) {
    if (!answers.some((x) => x.answer === a.answer)) answers.push(a);
  }
  const infoboxes = [...base.infoboxes];
  for (const ib of extra.infoboxes) {
    if (!infoboxes.some((x) => x.title === ib.title && x.content === ib.content)) infoboxes.push(ib);
  }
  const uniq = (items: string[]) => [...new Set(items.map((s) => s.trim()).filter(Boolean))];
  return {
    answers: answers.slice(0, 5),
    infoboxes: infoboxes.slice(0, 3),
    corrections: uniq([...base.corrections, ...extra.corrections]).slice(0, 3),
    suggestions: uniq([...base.suggestions, ...extra.suggestions]).slice(0, 8),
  };
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

export type EvidenceAssessment =
  | 'exact_excerpt'
  | 'lexical_match'
  | 'model_assessed'
  | 'unresolved'
  | 'conflicting';

export interface VerifiedClaim {
  claim: string;
  status: ClaimStatus;
  sources: string[];
  explanation?: string;
  /** How the status was decided. Lexical overlap is never a certification. */
  assessment?: EvidenceAssessment;
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
  /** Pages recovered from the Wayback Machine after a failed live fetch. */
  waybackRecoveries?: number;
  /** Chronic failing domains whose live fetch was skipped in favor of Wayback/snippets. */
  domainsChronicSkipped?: string[];
  /** Domains reordered to the back of the fetch queue due to past failures. */
  domainsDeprioritized?: string[];
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
  /** SearXNG direct answers / infoboxes / corrections / suggestions for this session. */
  searchMeta?: SearchMeta;
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
