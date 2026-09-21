# Phase 1 web grounding in LocalLM

The existing TypeScript engine in `src/web/` runs through the existing Node worker, outside the WebView. `src-tauri/src/web_search.rs` supplies the selected local model connection and settings; the harness exposes `web_search` in Standard, Research, Creator and Code presets. Chat-only does not offer it. This implementation adds no language, runtime, paid service or Docker dependency.

## Pipeline and defaults

`router/` → `planning/` → `search/` → `ranking/` → `fetch/` and `scripts/web-transport.mjs` → `extraction/` → `chunking/` → `retrieval/bm25.ts` → `evidence/` → `context/` → `generation/`.

The router is deterministic. The local model receives a separate schema-constrained planning request, then a synthesis request containing compressed evidence. Invalid planner output falls back to the original question, preserving version punctuation. With no local model, deterministic planning and attributed extracts remain available. These extracts are not a model-generated answer.

Normal mode allows up to 4 queries, 8 pages, 600-token paragraph-aware chunks with 80-token overlap, 2,500 evidence tokens and 6,000 total input tokens. BM25 is enabled; embeddings, reranking, model evidence extraction and model verification are disabled by default. Existing later-phase modules remain opt-in and are not established as production-ready by Phase 1 tests. The hashing embedding and heuristic reranker are not trained neural models.

Sources and citation IDs are application-owned. `web_open` and `web_find` expose only the selected evidence, not raw document slices. Full pages remain outside model context. Diagnostics print planned queries, raw and unique results, fetched pages, chunks, evidence/prompt counts and compression ratio.

## Run from the app

1. Configure Node.js 22.13+ in Execution settings and load a local model.
2. Run your self-hosted SearXNG with JSON output enabled. The default endpoint is `http://127.0.0.1:8080`; use another port if the model occupies it. No Docker? SearXNG also runs inside WSL without it: `git clone --depth 1 https://github.com/searxng/searxng && uv venv ~/sxng-venv && uv pip install --python ~/sxng-venv/bin/python -r ~/searxng/requirements.txt`, then `cd ~/searxng && SEARXNG_SETTINGS_PATH=/mnt/c/…/LocalLM/searxng/settings.yml PYTHONPATH=~/searxng ~/sxng-venv/bin/python -m searx.webapp`. WSL forwards it to `127.0.0.1:8080` on Windows.
3. Select Research (or another preset offering `web_search`) and ask a current-information question. Normal harness tool permission settings apply.

For overrides, create `%APPDATA%/app.locallm.desktop/web-search.json`. Example:

```json
{
  "searxngBaseUrl": "http://127.0.0.1:8080",
  "searchProvider": "searxng",
  "googleApiKey": "",
  "googleCxId": "",
  "searchFallback": { "enabled": true, "googleDailyLimit": 90 },
  "queries": { "fast": 2, "normal": 4, "deep": 6 },
  "fetch": { "normalPages": 8, "deepPages": 20, "deepTimeoutSeconds": 20, "deepMaxBytes": 10485760, "globalConcurrency": 4, "perDomainConcurrency": 2, "userAgents": [], "jsRenderFallback": false, "jsRenderTimeoutMs": 20000 },
  "chunking": { "targetTokens": 600, "overlapTokens": 80 },
  "context": { "evidenceTokenBudget": 2500, "totalInputBudget": 6000 },
  "extractEvidenceWithModel": false,
  "retrieval": { "bm25": true, "embeddings": false },
  "reranking": { "enabled": false },
  "verification": { "enabled": false, "maxResearchRetries": 1 },
  "cache": { "enabled": false }
}
```

The native bridge preserves context overrides while capping them to the loaded model context, reserving output space. Configuration interfaces and remaining tunables are in `src/web/config/schema.ts` and `defaults.ts`.

## CLI and tests

From the repository root, with your services already running:

```powershell
$env:LOCAL_LLM_BASE_URL = 'http://127.0.0.1:8081/v1'
$env:LOCAL_LLM_MODEL = 'your-loaded-model-id'
npm run research -- ask 'What is the latest stable React release?' --trace --sources
# Optional custom SearXNG/config file:
npm run research -- ask 'What changed in the latest Node.js release?' --config web-search.json --trace
npm run research -- health
```

The CLI uses the supplied loopback local model endpoint. The native app additionally passes its managed model authentication. SearXNG requires `json` under `search.formats`; see the [official Search API documentation](https://docs.searxng.org/dev/search_api).

```powershell
npm run test:grounding
npx vitest run src/web --maxWorkers=2
npm run test:web-transport
npm run build
cargo test --manifest-path src-tauri/Cargo.toml web_search::tests --lib
```

`test:grounding` uses a local fixture SearXNG JSON service and fetches real public HTTPS pages. It does not establish that React 19 is currently the newest release: its search fixture intentionally references historical release pages. Without `LOCAL_LLM_BASE_URL`, it tests the deterministic fallback, not model inference.

Recorded fixture run on 2026-09-17: 2 queries, 10 raw results, 3 unique public results, 3 pages fetched, 98 chunks, 17,661 estimated extracted tokens, 320 evidence tokens, 556 final prompt tokens, 55.2× compression. All smoke assertions passed. The local SearXNG health check returned `unreachable`; a real SearXNG-plus-model run remains unverified on this machine.

## Security and measurement boundaries

Public pages use DNS resolution plus socket address pinning, reject non-public addresses, validate every redirect and respect robots policies. Configured SearXNG and local inference are explicit trusted service endpoints, separate from public-page fetching; they do not follow redirects. HTTP 429/503 Retry-After establishes a cooldown. Failed pages are isolated; available search snippets can be used as fallback evidence and are marked in document metadata. The opt-in `jsRenderFallback` renders failed, challenged or thin pages with headless Edge in the worker (temporary profile, bounded DOM output, hard timeout); it repeats the public-address check before spawning because browser DNS bypasses the pinned fetch path, and it never runs for robots.txt refusals.

Web text is untrusted data. Known directives are neutralized, model output is checked against supplied evidence and application-owned citations, and research model calls have no tools or credentials in their prompts. Pattern matching and lexical verification do not prove immunity to every prompt injection or prove semantic truth. Source claims may themselves be wrong, stale or incomplete.

Without a model, token counts are planning estimates. With a local tokenizer, the context builder checks runtime counts and drops whole claims until both budgets fit. Without a tokenizer endpoint, the local provider uses conservative UTF-8 byte counts. The entire user message counts against the evidence limit conservatively; prompt accounting reserves 64 chat-template tokens and the configured safety margin. Unsupported or oversized inference fails back to attributed evidence instead of sending oversized pages.
