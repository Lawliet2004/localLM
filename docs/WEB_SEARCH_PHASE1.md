# Phase 1 web grounding in LocalLM

The existing TypeScript engine in `src/web/` runs through the existing Node worker, outside the WebView. `src-tauri/src/web_search.rs` supplies the selected local model connection and settings; the harness exposes `web_search` in Standard, Research, Creator and Code presets. Chat-only does not offer it. This implementation adds no language, runtime, paid service or Docker dependency.

## Pipeline and defaults

`router/` → `planning/` → `search/` → `ranking/` → `fetch/` and `scripts/web-transport.mjs` → `extraction/` → `chunking/` → `retrieval/bm25.ts` → `evidence/` → `context/` → `generation/`.

The router is deterministic. Query planning is rules-first: `planning: 'auto'` (the default) plans with deterministic query shaping on fast and normal turns and only spends one schema-constrained model call in deep mode; `planning: 'rules'` never calls the model and `planning: 'model'` keeps the previous always-model behavior. Invalid planner output falls back to the original question, preserving version punctuation. With no local model, deterministic planning and attributed extracts remain available. These extracts are not a model-generated answer.

Mode budgets follow the bounded controller table: fast allows 1 query and 3 pages, normal 2 queries and 6 pages, deep 4 queries and 10 pages; direct answers and verticals cost zero searches. Chunks remain 600-token paragraph-aware with 80-token overlap, 2,500 evidence tokens and 6,000 total input tokens. BM25 is enabled; embeddings, reranking, model evidence extraction and model verification are disabled by default. Existing later-phase modules remain opt-in and are not established as production-ready by Phase 1 tests. The hashing embedding and heuristic reranker are not trained neural models.

Reasoning control is detected, not assumed. The worker probes llama.cpp `/props` once per run and sends `reasoning_effort: 'low'` or `enable_thinking: false` on mechanical calls (planning, evidence extraction, claim verification) only when the loaded chat template advertises that kwarg. On the native side, finalizing rounds inspect the GGUF `tokenizer.chat_template` metadata instead of a model filename list, so any model that honors `enable_thinking` or `reasoning_effort` gets the suppression it understands; models without controls are untouched.

Sources and citation IDs are application-owned. `web_open` and `web_find` expose only the selected evidence, not raw document slices. Full pages remain outside model context. Diagnostics print planned queries, raw and unique results, fetched pages, chunks, evidence/prompt counts and compression ratio.

## Run from the app

1. Configure Node.js 22.13+ in Execution settings and load a local model.
2. Run your self-hosted SearXNG with JSON output enabled. The default endpoint is `http://127.0.0.1:8080`; use another port if the model occupies it. No Docker? SearXNG also runs inside WSL without it: `git clone --depth 1 https://github.com/searxng/searxng && uv venv ~/sxng-venv && uv pip install --python ~/sxng-venv/bin/python -r ~/searxng/requirements.txt`, then `cd ~/searxng && SEARXNG_SETTINGS_PATH=/mnt/c/…/LocalLM/searxng/settings.yml PYTHONPATH=~/searxng ~/sxng-venv/bin/python -m searx.webapp`. WSL forwards it to `127.0.0.1:8080` on Windows.
3. Select Research (or another preset offering `web_search`) and ask a current-information question. Normal harness tool permission settings apply.

With Docker Compose installed, the bundled configuration can be started explicitly with `docker compose up -d searxng`. It uses the pinned image in `docker-compose.yml`, binds only to loopback, and enables JSON. Engine selection lives in `searxng/settings.yml`; the client uses the instance defaults. Advanced `searxngEngines` and `searxngDisabledEngines` overrides are optional and default to empty arrays.

In **Settings → Web Search** (also available in Execution), enter the instance URL and click **Check connection**. This checks the current form without saving it and sends a real JSON search probe. A reverse-proxy prefix such as `https://search.example/tools/searx/` is preserved; a pasted `/search` endpoint is also accepted. Multiple instances can be separated with commas or semicolons. Failover shares one timeout budget, reserving time for each remaining instance; HTTP 429/503 cooldowns apply independently to each instance within the worker.

The separate Web Search destination controls enablement and the default **Fast**, **Balanced** (`normal` internally) or **Deep** budget through the same pipeline. JSON/CSV/TSV fetches use bounded structured extraction, alongside existing HTML/PDF text extraction. Persistent cache storage is TTL-pruned and bounded to 2,000 entries/64 MiB serialized data; entries over 10 MiB are not cached. Weather has a separate 64-entry cache. **Clear retrieval cache** also clears weather cache, preserves saved research/citation history, and works offline or while research is disabled.

Connection feedback distinguishes usable search results (`healthy`), results with engine failures (`degraded`), a valid empty response (`empty`), API/configuration errors (`error`), and transport failures (`unreachable`). HTTP 403 and HTML responses include JSON setup guidance. Engine failures arrive as SearXNG's `[engine, reason]` tuples and remain visible in diagnostics. An empty result accompanied by engine failures is reported as a failed query, rather than a successful empty search. Permanent configuration errors are not retried. Click **Save search settings** when the connection is configured.

For overrides, create `%APPDATA%/app.locallm.desktop/web-search.json`. Example:

```json
{
  "searxngBaseUrl": "http://127.0.0.1:8080",
  "searchProvider": "searxng",
  "googleApiKey": "",
  "googleCxId": "",
  "searchFallback": { "enabled": true, "googleDailyLimit": 90 },
  "planning": "auto",
  "queries": { "fast": 1, "normal": 2, "deep": 4 },
  "fetch": { "fastPages": 3, "normalPages": 6, "deepPages": 10, "deepTimeoutSeconds": 20, "deepMaxBytes": 10485760, "globalConcurrency": 4, "perDomainConcurrency": 2, "userAgents": [], "jsRenderFallback": false, "jsRenderTimeoutMs": 20000 },
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

Environment variables applied last in `createConfig` (they win over `web-search.json` so a deployment can pin them): `SEARXNG_BASE_URL`, `SEARXNG_TIMEOUT_MS` (1000–120000), `SEARXNG_ENGINES` and `SEARXNG_DISABLED_ENGINES` (comma-separated), and `AGC_WEB_PROFILE=low_memory`. `LOCAL_LLM_BASE_URL`/`LOCAL_LLM_MODEL` are CLI-only model overrides.

The `web_search` tool also accepts optional `freshness` (`day|week|month|year|realtime`), `language`, and `category` arguments, which are validated and passed through to every SearXNG query in the turn. During research, the worker emits `\x1e`-prefixed JSON progress events on stderr that the native bridge relays as chat activity: route selection, query count, result count, per-page fetch progress, evidence count, and answer synthesis.

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
node --test scripts/searxng-worker.test.mjs
node scripts/web-benchmark.mjs --json bench-results.json
cargo test --manifest-path src-tauri/Cargo.toml web_search::tests --lib
```

`test:grounding` uses a local fixture SearXNG JSON service and fetches real public HTTPS pages. It does not establish that React 19 is currently the newest release: its search fixture intentionally references historical release pages. Without `LOCAL_LLM_BASE_URL`, it tests the deterministic fallback, not model inference.

`searxng-worker.test.mjs` exercises the built worker against real loopback HTTP fixtures: JSON health, proxy paths, multiple-instance failover, concurrent query metadata, forbidden/HTML responses, and upstream engine failures. It does not require a running SearXNG instance or model.

Recorded fixture run on 2026-09-17: 2 queries, 10 raw results, 3 unique public results, 3 pages fetched, 98 chunks, 17,661 estimated extracted tokens, 320 evidence tokens, 556 final prompt tokens, 55.2× compression. All smoke assertions passed. The local SearXNG health check returned `unreachable`; a real SearXNG-plus-model run remains unverified on this machine.

## Security and measurement boundaries

Public pages use DNS resolution plus socket address pinning, reject non-public addresses, validate every redirect and respect robots policies. Configured SearXNG and local inference are explicit trusted service endpoints, separate from public-page fetching; they do not follow redirects. HTTP 429/503 Retry-After establishes a cooldown. Failed pages are isolated; available search snippets can be used as fallback evidence and are marked in document metadata. The opt-in `jsRenderFallback` renders failed, challenged or thin pages with headless Edge in the worker (temporary profile, bounded DOM output, hard timeout); it repeats the public-address check before spawning because browser DNS bypasses the pinned fetch path, and it never runs for robots.txt refusals.

Web text is untrusted data. Known directives are neutralized, model output is checked against supplied evidence and application-owned citations, and research model calls have no tools or credentials in their prompts. Pattern matching and lexical verification do not prove immunity to every prompt injection or prove semantic truth. Source claims may themselves be wrong, stale or incomplete.

Without a model, token counts are planning estimates. With a local tokenizer, the context builder checks runtime counts and drops whole claims until both budgets fit. Without a tokenizer endpoint, the local provider uses conservative UTF-8 byte counts. The entire user message counts against the evidence limit conservatively; prompt accounting reserves 64 chat-template tokens and the configured safety margin. Unsupported or oversized inference fails back to attributed evidence instead of sending oversized pages.
