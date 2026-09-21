# Local-First Web Search & Grounding Engine for Small LLMs (5B–10B)

> For the implemented Phase 1 defaults, setup, measured tests and limitations, read [WEB_SEARCH_PHASE1.md](WEB_SEARCH_PHASE1.md). The broader design below includes optional later phases; its production-quality and model descriptions are not acceptance evidence.

A production-quality, zero-cost, local-first web search, retrieval, grounding, citation, and verification engine designed for small local language models (5B–10B parameters, such as MiniCPM5-2B, Qwen 2.5 7B, Llama 3.1 8B, and Mistral 7B).

---

## 1. Fundamental Design Principle: Search Space vs. LLM Context

Small language models struggle when flooded with dozens of raw webpages, sprawling context windows, or complex tool execution logic. This engine enforces a strict physical separation:

```text
SEARCH SPACE (Outside LLM Context)                     ACTIVE LLM REASONING CONTEXT
-----------------------------------                     ----------------------------
10 search queries                                       Current Date
80 SERP results                                         User Question
20 webpages fetched safely                              Structured Vertical Data (if any)
100,000+ tokens extracted                               Distilled Grounded Evidence (~1.5K–2.5K tokens)
BM25 + Dense Vector In-Memory Index                     Verified Source Map ([S1], [S2])
Cross-Encoder Rerank (Top 25 -> Top 8)                  Conflict Notes (if any)
Factual Evidence Extraction & Compression
Strict Token Budget Allocation
```

By offloading searching, crawling, filtering, deduplication, chunking, retrieval, reranking, and citation mapping into deterministic algorithms, the small local LLM receives only a small package of relevant, grounded facts.

---

## 2. End-to-End Pipeline Architecture

```text
                              USER QUESTION
                                    │
                                    ▼
                         ┌────────────────────┐
                         │ REQUEST NORMALIZER │
                         └─────────┬──────────┘
                                   │
                                   ▼
                         ┌────────────────────┐
                         │  INTENT / FRESHNESS│
                         │      ROUTER        │
                         └─────────┬──────────┘
                                   │
             ┌─────────────────────┼──────────────────────┐
             │                     │                      │
             ▼                     ▼                      ▼
      STATIC KNOWLEDGE      STRUCTURED DATA          WEB SEARCH
          (NONE)               VERTICALS               PIPELINE
                          (Weather, FX, Time)             │
             │                     │                      ▼
             │                     │             ┌────────────────┐
             │                     │             │ QUERY PLANNER  │
             │                     │             │  (1–4 queries) │
             │                     │             └────────┬───────┘
             │                     │                      │
             │                     │                      ▼
             │                     │             ┌────────────────┐
             │                     │             │ SEARCH SERVICE │
             │                     │             │ (SearXNG API)  │
             │                     │             └────────┬───────┘
             │                     │                      │
             │                     │                      ▼
             │                     │             ┌────────────────┐
             │                     │             │ RESULT FUSION  │
             │                     │             │ DEDUP + SCORE  │
             │                     │             └────────┬───────┘
             │                     │                      │
             │                     │                      ▼
             │                     │             ┌────────────────┐
             │                     │             │  PAGE FETCHER  │
             │                     │             │  (SSRF Guard)  │
             │                     │             └────────┬───────┘
             │                     │                      │
             │                     │                      ▼
             │                     │             ┌────────────────┐
             │                     │             │ MAIN EXTRACTION│
             │                     │             │ & FINGERPRINT  │
             │                     │             └────────┬───────┘
             │                     │                      │
             │                     │                      ▼
             │                     │             ┌────────────────┐
             │                     │             │SEMANTIC CHUNKER│
             │                     │             └────────┬───────┘
             │                     │                      │
             │                     │                      ▼
             │                     │            ┌─────────────────┐
             │                     │            │ HYBRID RETRIEVAL│
             │                     │            │ (BM25 + Dense)  │
             │                     │            └─────────┬───────┘
             │                     │                      │
             │                     │                      ▼
             │                     │            ┌─────────────────┐
             │                     │            │  CROSS-ENCODER  │
             │                     │            │    RERANKER     │
             │                     │            └─────────┬───────┘
             │                     │                      │
             │                     │                      ▼
             │                     │            ┌─────────────────┐
             │                     │            │EVIDENCE EXTRACT │
             │                     │            │& CONFLICT DETECT│
             │                     │            └─────────┬───────┘
             │                     │                      │
             └─────────────────────┼──────────────────────┘
                                   │
                                   ▼
                         ┌────────────────────┐
                         │  CONTEXT BUILDER   │
                         │ & TOKEN BUDGETING  │
                         └─────────┬──────────┘
                                   │ ~2,000 evidence tokens
                                   ▼
                         ┌────────────────────┐
                         │  LOCAL SMALL LLM   │
                         │  ANSWER SYNTHESIS  │
                         └─────────┬──────────┘
                                   │
                                   ▼
                         ┌────────────────────┐
                         │ CITATION VALIDATOR │
                         │   & CLAIM AUDITOR  │
                         └─────────┬──────────┘
                                   │
                      ┌────────────┴─────────────┐
                      ▼                          ▼
                  SUPPORTED                 UNSUPPORTED
                      │                          │
                 FINAL ANSWER             TARGETED RETRY
```

---

## 3. Zero-Paid-API Operational Model

The default implementation does not require any paid commercial APIs:
- **Search Provider:** Self-hosted SearXNG running via Docker Compose (`docker-compose.yml`) exposing a local JSON endpoint.
- **Weather Vertical:** Open-Meteo REST API + OpenStreetMap Nominatim geocoding (zero cost, no API keys).
- **Currency Vertical:** Open Exchange Rates free endpoint (`https://open.er-api.com/`).
- **Time Vertical:** Deterministic system clock + standard IANA timezone database.
- **Lexical Retrieval:** In-memory Okapi BM25 implementation.
- **Dense Retrieval:** 1024-dimensional feature hashing and character n-gram dense unit vectorizer with cosine similarity.
- **Reranking:** Local cross-encoder passage scorer with pairwise phrase alignment and term proximity scoring.
- **Persistence & Cache:** SQLite schema (`search_cache`, `document_cache`, `embedding_cache`) with TTL expiration.
- **Inference Runtime:** Local OpenAI-compatible loopback server (`llama.cpp`, `Ollama`, `vLLM`, or `LM Studio`).

---

## 4. Security Architecture

### SSRF (Server-Side Request Forgery) Guard
All external page fetches are filtered through `SSRFGuard`:
- Blocks `127.0.0.1`, `localhost`, `0.0.0.0`, `::1`.
- Blocks private IPv4 blocks: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`.
- Blocks link-local & cloud metadata IPs: `169.254.0.0/16`, `169.254.169.254`, `metadata.google.internal`.
- Blocks internal TLDs: `.local`, `.internal`, `.localhost`, `.corp`, `.lan`.
- Re-verifies every hop in redirect chains.
- Enforces strict `http:` and `https:` protocols (rejects `file://`, `gopher://`, `ftp://`).

### Prompt Injection Defense
Webpages are treated as untrusted data. The `InjectionGuard`:
- Neutralizes prompt injection phrases (`IGNORE ALL PREVIOUS INSTRUCTIONS`, `SYSTEM PROMPT:`, etc.).
- Wraps extracted passages in explicit untrusted boundary markers.
- Informs synthesis prompts that instructions inside source passages are unprivileged and cannot alter system behavior.

---

## 5. Directory & Module Structure

```text
src/web/
├── index.ts                      # Master WebSearchEngine orchestrator
├── types.ts                      # Core domain types (SearchRequest, RetrievedDocument, etc.)
├── tools.ts                      # Progressive disclosure agent tools (web_search, weather, web_open, web_find)
├── router/
│   ├── intent_router.ts          # Intent detection & complexity classification
│   ├── freshness.ts              # Temporal intent detection (realtime, day, week, month, year)
│   └── vertical_router.ts        # Route to structured verticals (WEATHER, CURRENCY, TIME)
├── planning/
│   ├── query_planner.ts          # Generates 1–4 targeted queries
│   ├── query_schema.ts           # JSON schema for query planning
│   └── fallback_planner.ts       # Deterministic query fallback
├── search/
│   ├── provider.ts               # SearchProvider interface & MockProvider
│   ├── searxng_provider.ts       # SearXNG HTTP JSON provider
│   ├── search_service.ts         # Concurrency control & failure isolation
│   ├── result_normalizer.ts      # Standardizes SERP items
│   └── result_fusion.ts          # Pools candidate results & runs deduplication
├── ranking/
│   ├── url_normalizer.ts         # Strips tracking (utm_*, fbclid), normalizes URLs
│   ├── deduplicator.ts           # URL/canonical equality & Token Jaccard title similarity
│   ├── source_quality.ts         # Domain classification (docs, academic, news, forum)
│   ├── freshness_score.ts        # Intent-aware exponential decay
│   └── search_ranker.ts          # Multi-factor ranking formula
├── fetch/
│   ├── http_fetcher.ts           # Safe fetcher with timeouts, byte limits, redirect checks
│   ├── fetch_policy.ts           # Page count policy (fast: 4, normal: 8, deep: 16)
│   ├── robots_policy.ts          # Robots.txt parser and path compliance
│   └── rate_limiter.ts           # Bounded global & per-domain concurrency
├── extraction/
│   ├── main_content.ts           # Boilerplate stripping & Markdown extraction
│   ├── metadata.ts               # OpenGraph, JSON-LD, article date/author extractor
│   └── extraction_fallback.ts    # Fallback routines for sparse content
├── chunking/
│   ├── semantic_chunker.ts       # Chunking on headings, paragraphs, lists with overlap
│   └── tokenizer.ts              # Token counting estimator
├── retrieval/
│   ├── bm25.ts                   # In-memory Okapi BM25 implementation
│   ├── embeddings.ts             # 1024-dim dense hashing vectorizer & cosine similarity
│   ├── rrf.ts                    # Reciprocal Rank Fusion algorithm
│   ├── diversity.ts              # Per-doc limits and Maximal Marginal Relevance (MMR)
│   └── hybrid_retriever.ts       # Coordinates BM25 + Embeddings + RRF + Diversity
├── reranking/
│   ├── cross_encoder.ts          # Pairwise passage relevance reranker
│   └── reranker.ts               # Reranker service with low-RAM fallback
├── evidence/
│   ├── source_registry.ts        # Deterministic source IDs (S1, S2, ...) and chunk mapping
│   ├── extractor.ts              # Atomic fact extraction with schema validation
│   ├── deduplicator.ts           # Fact clustering & multi-source attribution
│   └── conflict_detector.ts      # Disagreement and contradiction detection
├── context/
│   ├── context_builder.ts        # Compact evidence package construction
│   └── token_budget.ts           # Strict token budget allocation (never truncates mid-sentence)
├── generation/
│   ├── answer_generator.ts       # Grounded answer synthesis
│   └── citation_renderer.ts      # Citation validator, cleaner, and Markdown renderer
├── verification/
│   ├── claim_extractor.ts        # Decomposes answer into atomic claims
│   ├── claim_verifier.ts         # Verifies claims against evidence (SUPPORTED, CONFLICTING)
│   └── research_retry.ts         # Targeted retry evaluator for unsupported claims
├── verticals/
│   ├── vertical_provider.ts      # VerticalProvider interface
│   ├── weather/                  # Open-Meteo zero-cost structured weather
│   ├── currency/                 # Free currency converter
│   └── time/                     # World time & timezone resolver
├── cache/
│   ├── sqlite.ts                 # SQLite cache tables schema & storage adapter
│   ├── search_cache.ts           # Search query cache
│   ├── document_cache.ts         # Extracted document cache
│   └── embedding_cache.ts        # Vector embedding cache
├── security/
│   ├── ssrf_guard.ts             # SSRF protection and IP validation
│   ├── injection_guard.ts        # Prompt injection containment
│   ├── url_policy.ts             # Protocol & URL validation
│   └── content_sanitizer.ts      # HTML tag and XSS sanitizer
├── observability/
│   ├── logger.ts                 # Structured stage logger
│   ├── trace.ts                  # Quantitative metrics collector
│   └── metrics.ts                # Diagnostic report generator
├── models/
│   ├── llm_provider.ts           # LLMProvider interface & MockLLMProvider
│   └── local_provider.ts         # OpenAI-compatible local server provider
├── config/
│   ├── schema.ts                 # WebSearchConfig types
│   └── defaults.ts               # Default configurations & low_memory profile
└── evaluation/
    ├── dataset.ts                # 52 benchmark evaluation questions across 13 categories
    └── evaluator.ts              # Automated evaluation harness
```

---

## 6. Quickstart: Running SearXNG with Docker Compose

To start the local SearXNG search backend:

```bash
docker compose up -d
```

Verify SearXNG is healthy:

```bash
curl "http://127.0.0.1:8080/search?q=test&format=json"
```

With `use_default_settings: true`, SearXNG's full upstream catalogue (~260 engines) loads and the client sends an `engines` whitelist per query (see `searxngEngines` in `src/web/config/defaults.ts`). The bundled `searxng/settings.yml` only pins overrides: it enables `yep` and disables `google`, `bing`, `yandex`, `brave`, `qwant`. General results ride on `google cse` (key-free, enabled by default upstream), `yep`, `duckduckgo` (intermittent CAPTCHA), `google news`, `reuters`, `wikipedia`/`wikinews`, plus the niche engines. `mojeek`/`startpage` are `inactive` upstream (proof-of-work CAPTCHA) and cannot be enabled. The smoke test above should return a non-empty `results` array with multiple engine names in the response. After editing `settings.yml`, restart the SearXNG process/container (`docker compose up -d` when using Docker).

---

## 7. Programmatic Usage Example

```typescript
import { WebSearchEngine } from './src/web';

const engine = new WebSearchEngine({
  config: {
    mode: 'normal',
    searxngBaseUrl: 'http://127.0.0.1:8080',
  },
});

// Run research turn
const session = await engine.research('What changed in React 19 recently?');

console.log(session.answer);
// Prints grounded answer with clickable markdown citations [[S1]](https://react.dev/...)
```

---

## 8. Verification & Diagnostics Output

Every research turn records complete operational metrics:

```text
--- RESEARCH DIAGNOSTICS REPORT ---
Question: What is the newest stable release of Next.js?
Routing: GENERAL_WEB (medium, Freshness: month)
Queries: 2
Raw results: 20
Unique results: 14
Pages selected: 8
Pages successfully fetched: 8 (Failures: 0)
Extracted tokens: 54,200
Chunks: 92
Hybrid candidates: 25
Reranked chunks: 8
Evidence statements: 11
Evidence tokens passed to final LLM: 1,840
Total final prompt tokens: 2,980
Information Compression Ratio: 29.5x (examined ~54,200 tokens -> sent ~1,840 tokens)
Citations rendered: 4
Verified factual claims: 11/11
Unsupported claims: 0
Conflicting claims: 0
Total latency: 1420ms
-----------------------------------
```

---

## 9. Evaluation Dataset & Benchmark Harness

An internal dataset of **52 questions** across all 13 required categories is defined in `src/web/evaluation/dataset.ts`:
1. Weather (W1–W5)
2. Recent News (N1–N4)
3. Software Version (SV1–SV5)
4. Technical Documentation (TD1–TD4)
5. Benchmark Comparison (BC1–BC5)
6. Historical Facts / Static Knowledge (HF1–HF4)
7. Multi-Source Research (MR1–MR4)
8. Ambiguous Queries (AQ1–AQ4)
9. Conflicting Sources (CS1–CS4)
10. Missing Information / Hallucination Resistance (MI1–MI4)
11. Bad / Broken Webpages (BW1–BW4)
12. Duplicate / Syndicated Sources (DS1–DS4)
13. Search Provider Failure & Prompt Injection (PF1, PI1–PI3)

Run the evaluation suite:

```bash
npx vitest run src/web/evaluation/evaluator.test.ts
```
