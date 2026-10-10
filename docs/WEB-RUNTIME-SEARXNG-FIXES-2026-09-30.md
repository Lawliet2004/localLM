# Web/runtime/SearXNG fixes — 2026-09-30

All A1–A23 and ten demonstrated Part B findings are fixed. No commits, pushes, dependency additions, or user-data deletions were made. Existing unrelated working-tree changes were preserved; the task-start source baseline and command logs are under `.local/web-runtime-fixes-20260930/`. The generated worker was rebuilt from source.

## Test evidence conventions

“Red → green” means the maintained regression was run against the defective implementation, observed failing, and then passed after the fix. Tests extending existing cases are explicitly identified. Secondary coverage added after implementation is labeled green-only; code-trace and native-loader findings are distinguished from assertion failures. The final full suites include these tests.

## A1 — Offline retrieval boundary

- **Status:** fixed. Network actions, including unknown actions and full research, fail before worker installation or spawn. Stored open/find/verify are exempt from the offline gate. Weather also uses the shared retrieval policy.
- **Changed locations:** [src-tauri/src/web_search.rs:77](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:77>); [src-tauri/src/web_search.rs:201](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:201>); [src-tauri/src/chat.rs:1306](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/chat.rs:1306>).
- **Maintained regression:** [src-tauri/src/web_search.rs:313](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:313>) — `offline_blocks_network_before_worker_install_but_allows_stored_actions`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** The table-driven test covers action classification and confirms no worker is installed. Stored actions are tested for exemption from the offline error, not live stored-session integration. Offline is a retrieval policy; permissioned shell/MCP are not network-isolated.

## A2 — Keyless loopback snapshots and API format

- **Status:** fixed. Snapshots enforce local-only inference, accept keyless loopback OpenAI-compatible providers, preserve the Claude key requirement, and retain API format separately from network location. The worker accepts explicit local provider URLs.
- **Changed locations:** [src-tauri/src/subagents.rs:219](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/subagents.rs:219>); [src-tauri/src/subagents.rs:318](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/subagents.rs:318>); [src-tauri/src/web_search.rs:112](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:112>).
- **Maintained regression:** [src-tauri/src/web_search.rs:333](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:333>) — `keyless_loopback_snapshot_succeeds_but_remote_and_keyless_claude_fail`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Snapshot behavior is covered with real temporary stores. Worker selection is established by the local kind/API-format gate and code trace; no real provider/model integration was run.

## A3 — Per-slot context allocation

- **Status:** fixed. The configured context is per slot: --ctx-size emits N*S, KV memory estimation remains N*S, and stored runtime capacity prefers authenticated /props default_generation_settings.n_ctx with N as fallback. Trained-model maximum validation still compares N.
- **Changed locations:** [src-tauri/src/runtime_config.rs:94](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/runtime_config.rs:94>); [src-tauri/src/runtime.rs:271](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/runtime.rs:271>).
- **Maintained regression:** [src-tauri/src/runtime_config.rs:202](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/runtime_config.rs:202>) — `context_allocation_is_per_slot_and_matches_kv_estimate`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** S=1/S=2 allocation and memory consistency are covered. Runtime /props measurement has not been validated against a live two-slot model.

## A4 — Grounded AREX visit briefs

- **Status:** fixed. Model fact lines require normalized page-substring membership and matching numeric literals; ungrounded output falls back to deterministic extraction. Source URLs remain application-owned.
- **Changed locations:** [src/web/extraction/result_extractor.ts:78](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/extraction/result_extractor.ts:78>).
- **Maintained regression:** [src/web/extraction/result_extractor.test.ts:12](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/extraction/result_extractor.test.ts:12>) — `rejects invented model facts and retains verbatim page lines`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Substring checks are conservative and can reject useful paraphrases. They establish provenance, not whether the source itself is true.

## A5 — Synthesis polarity

- **Status:** fixed. Synthesis must preserve the cited evidence polarity markers as well as existing numeric/comparative/overlap constraints; otherwise attributed fallback is returned.
- **Changed locations:** [src/web/staged_research.ts:242](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/staged_research.ts:242>).
- **Maintained regression:** [src/web/staged_research.test.ts:33](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/staged_research.test.ts:33>) — `rejects synthesis with reversed polarity and accepts faithful text`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** This is a lexical check; complex scope, double negation, and semantic equivalence remain limitations.

## A6 — UTF-8 streaming

- **Status:** fixed. Worker stdin uses streaming UTF-8 decoding and a 64,000-byte cap. CLI stdout and the provider smoke HTTP request use setEncoding before string accumulation.
- **Changed locations:** [scripts/web-worker.mjs:47](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/web-worker.mjs:47>); [scripts/web-search-cli.mjs:31](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/web-search-cli.mjs:31>); [scripts/provider-connection-smoke.mjs:13](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/provider-connection-smoke.mjs:13>).
- **Maintained regression:** [scripts/worker-input.test.mjs:44](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/worker-input.test.mjs:44>) — `worker decodes UTF-8 characters split across stdin writes`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** The regression runs the regenerated embedded worker with a multibyte character split between writes. The initial stale-bundle run passed; rebuilding the defective source established the red result before the fix.

## A7 — Retain table evidence on prose failure

- **Status:** fixed. Table facts and their source entries are committed before the prose model call; prose failure diagnostics no longer discard valid table evidence.
- **Changed locations:** [src/web/staged_research.ts:377](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/staged_research.ts:377>).
- **Maintained regression:** [src/web/staged_research.test.ts:46](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/staged_research.test.ts:46>) — `retains table evidence when the prose model throws`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Available table evidence can be incomplete; failures remain visible in extraction diagnostics.

## A8 — Decode AREX query arrays

- **Status:** fixed. The harness forwards decoded queries with original casing/order and normalizes only the cache key. Standalone worker callers also receive JSON-text-array decoding.
- **Changed locations:** [src-tauri/src/harness.rs:341](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/harness.rs:341>); [scripts/web-worker.mjs:110](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/web-worker.mjs:110>).
- **Maintained regression:** [src-tauri/src/harness.rs:1046](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/harness.rs:1046>) — `arex_search_forwards_decoded_queries_in_original_order_and_case`; [scripts/worker-input.test.mjs:68](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/worker-input.test.mjs:68>) — `worker decodes JSON-text query arrays for standalone callers`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Natural-language query shortening remains intentional; explicit syntax is preserved by A9. Malformed JSON-text arrays return an error.

## A9 — Preserve explicit search syntax

- **Status:** fixed. Operators, excluded terms and quoted phrases pass through unchanged in compactQuery and the deterministic/model QueryPlanner entry point.
- **Changed locations:** [src/web/planning/fallback_planner.ts:42](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/planning/fallback_planner.ts:42>); [src/web/planning/query_planner.ts:12](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/planning/query_planner.ts:12>).
- **Maintained regression:** [src/web/planning/agent_search.test.ts:42](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/planning/agent_search.test.ts:42>) — `preserves explicit operators and exact phrases`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** The recognized syntax covers the requested operators; it is not a parser for every search engine dialect.

## A10 — Whole-page relevant excerpts

- **Status:** fixed. Goal terms rank sentence/chunk candidates across the whole sanitized page; selected chunks up to 12,000 characters feed both model extraction and deterministic fallback.
- **Changed locations:** [src/web/extraction/result_extractor.ts:26](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/extraction/result_extractor.ts:26>).
- **Maintained regression:** [src/web/extraction/result_extractor.test.ts:20](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/extraction/result_extractor.test.ts:20>) — `finds goal-relevant evidence beyond character 20000`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Term ranking can miss evidence requiring semantic matching; output and model input remain bounded.

## A11 — Two URLs per domain

- **Status:** fixed. Snippet filtering deduplicates by URL, allows two relevant results per domain, and retains the five-result cap.
- **Changed locations:** [src/web/staged_research.ts:93](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/staged_research.ts:93>).
- **Maintained regression:** [src/web/staged_research.test.ts:64](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/staged_research.test.ts:64>) — `keeps two distinct relevant URLs per domain and rejects a third`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** A third useful page on a domain is intentionally excluded by the bounded policy. The prior one-per-domain expectation was updated.

## A12 — SearXNG engine failures

- **Status:** fixed. Pair-form and legacy unresponsive engine diagnostics are parsed. Empty results with every requested engine failing throw with reasons; partial results remain usable. Health includes unresponsive engine failures.
- **Changed locations:** [src/web/search/searxng_provider.ts:135](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/searxng_provider.ts:135>).
- **Maintained regression:** [src/web/search/searxng_provider.test.ts:6](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/searxng_provider.test.ts:6>) — `counts all-unresponsive engines as a failure but retains partial results`; [src/web/search/searxng_provider.test.ts:16](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/searxng_provider.test.ts:16>) — `engine health reports unresponsive engine reasons`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Failure detection depends on SearXNG naming the failed engines. A genuinely empty, otherwise healthy search remains successful.

## A13 — Persistent Retry-After

- **Status:** fixed. SearXNG saves endpoint cooldown timestamps in the engine storage and checks them before search and health requests. Native and CLI workers use their shared research SQLite database.
- **Changed locations:** [src/web/search/searxng_provider.ts:214](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/searxng_provider.ts:214>); [src/web/index.ts:110](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/index.ts:110>).
- **Maintained regression:** [src/web/search/searxng_provider.test.ts:22](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/searxng_provider.test.ts:22>) — `persists Retry-After across providers sharing a storage`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Persistence requires shared storage. The untouched old worker repro explicitly uses :memory: per process and still passes; its requests intentionally cannot share cooldown state.

## A14 — Honest Google health

- **Status:** fixed. Google health reports configured_not_tested or not_configured and spends no search request. The UI explicitly labels the connection as untested.
- **Changed locations:** [src/web/index.ts:162](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/index.ts:162>); [src/components/Execution.tsx:150](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/components/Execution.tsx:150>).
- **Maintained regression:** [src/web/index.test.ts:9](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/index.test.ts:9>) — `Google health is explicitly untested and spends no query`; [src/components/WebSearchSettings.test.tsx:25](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/components/WebSearchSettings.test.tsx:25>) — `labels a configured Google connection as untested`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** The engine regression failed before/passes after. The additional UI test was added after the implementation and has only a green observation. Credentials and endpoint availability are not verified by this status.

## A15 — Atomic per-request Google quota

- **Status:** fixed. used+n must stay within the limit. SQLite uses one guarded UPSERT across connections; memory storage has an equivalent atomic claim. The Google provider claims immediately before each actual request, including retries and primary non-staged research.
- **Changed locations:** [scripts/web-storage.mjs:19](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/web-storage.mjs:19>); [src/web/cache/sqlite.ts:22](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/cache/sqlite.ts:22>); [src/web/search/google_provider.ts:35](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/google_provider.ts:35>).
- **Maintained regression:** [src/web/index.test.ts:17](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/index.test.ts:17>) — `Google quota refuses an oversized batch without changing the counter`; [src/web/index.test.ts:27](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/index.test.ts:27>) — `primary Google research counts every request including retries`; [scripts/worker-input.test.mjs:11](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/worker-input.test.mjs:11>) — `quota claims are atomic across SQLite connections and refuse oversized batches`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Both engine tests were red/green. The cross-connection SQLite test was added after the implementation and passed. The app enforces 90 per UTC date in its shared store; direct provider callers supplying no quota callback own their own limit.

## A16 — Google freshness and pagination

- **Status:** fixed. Freshness maps to d1/w1/m1/y1. Page controls start=(page-1)*num+1; num is capped to 10 and requests beyond result 100 are avoided.
- **Changed locations:** [src/web/search/google_provider.ts:23](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/google_provider.ts:23>).
- **Maintained regression:** [src/web/search/google_provider.test.ts:17](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/google_provider.test.ts:17>) — `maps freshness and page to Google request parameters`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Google freshness is an API filter, not a guarantee of publication accuracy.

## A17 — Actual provider provenance

- **Status:** fixed. Search batches expose provider, fallbackUsed, fallbackReason and primaryFailures. Staged traces aggregate the providers and fallback reasons that actually supplied evidence; worker search forwards this metadata.
- **Changed locations:** [src/web/index.ts:191](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/index.ts:191>); [src/web/index.ts:410](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/index.ts:410>); [scripts/web-worker.mjs:125](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/web-worker.mjs:125>).
- **Maintained regression:** [src/web/index.test.ts:37](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/index.test.ts:37>) — `staged fallback reports the provider that supplied evidence`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** A run can report multiple providers. Metadata describes retrieval provenance rather than independent corroboration.

## A18 — SearXNG path prefixes

- **Status:** fixed. Search and health resolve relative search paths under the configured base, retaining /searxng/ prefixes.
- **Changed locations:** [src/web/search/searxng_provider.ts:168](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/searxng_provider.ts:168>).
- **Maintained regression:** [src/web/search/searxng_provider.test.ts:35](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/searxng_provider.test.ts:35>) — `preserves base path prefixes for search and engine health`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Origin and sub-path URL construction are tested. No live reverse proxy was available.

## A19 — Fresh-install fallback default

- **Status:** fixed. Missing fallback settings use Option<bool> and default true; saved false/true remain authoritative in getter and worker configuration.
- **Changed locations:** [src-tauri/src/web_search.rs:88](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:88>).
- **Maintained regression:** [src-tauri/src/web_search.rs:275](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:275>) — `search_fallback_defaults_true_and_respects_saved_values`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Other scalar settings were audited; intentional zero/false defaults and explicit sentinel handling were left unchanged.

## A20 — Google credential vault and UI contract

- **Status:** fixed. The fixed web-search-google vault credential replaces plaintext settings. Legacy values migrate and are blanked after successful vault storage. The getter returns only hasGoogleApiKey; blank saves retain a key and explicit clear removes it.
- **Changed locations:** [src-tauri/src/web_search.rs:12](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:12>); [src-tauri/src/web_search.rs:235](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:235>); [src/lib/types.ts:201](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/lib/types.ts:201>); [src/lib/api.ts:191](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/lib/api.ts:191>); [src/components/Execution.tsx:21](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/components/Execution.tsx:21>).
- **Maintained regression:** [src-tauri/src/web_search.rs:284](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:284>) — `google_key_migrates_to_vault_and_is_never_returned_to_ui`; [src/components/WebSearchSettings.test.tsx:12](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/components/WebSearchSettings.test.tsx:12>) — `keeps saved credentials out of the input and makes clearing explicit`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Migration/getter regression was red/green; the additional UI test was added after implementation and passed. Execution.test.tsx mock shape was updated; e2e fixtures did not contain the old Google key field. User-authored web-search.json/CLI credential configuration is still caller-owned.

## A21 — Weather respects offered tools

- **Status:** fixed. The shortcut requires an offered web_search/get_weather alias, online retrieval, and non-plan mode; a preset harness flag alone no longer authorizes it.
- **Changed locations:** [src-tauri/src/chat.rs:1306](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/chat.rs:1306>); [src-tauri/src/chat.rs:1781](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/chat.rs:1781>).
- **Maintained regression:** [src-tauri/src/chat.rs:3578](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/chat.rs:3578>) — `fn weather_shortcut_requires_an_offered_tool_and_online_mode`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Pure gating covers empty/offered tools, offline and plan mode. Real weather retrieval was not invoked.

## A22 — Recent bounded claim-audit evidence

- **Status:** fixed. The evidence ring retains the 24 most recent unique items instead of stopping after the first 24. Audit fields use UTF-8 byte truncation and the final user audit note discloses possible earlier truncation.
- **Changed locations:** [src-tauri/src/arex.rs:155](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/arex.rs:155>); [src-tauri/src/arex.rs:190](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/arex.rs:190>); [src-tauri/src/chat.rs:531](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/chat.rs:531>).
- **Maintained regression:** [src-tauri/src/arex.rs:337](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/arex.rs:337>) — `collect_evidence_bounds_dedupes_and_skips_errors`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** The existing test was extended and observed red/green with 40 items. The retained count is known; original total is not tracked, so the note says earlier items may have been truncated. JSON escaping/configuration overhead can still reject unusually large requests safely at the worker cap.

## A23 — Nested managed-model slot

- **Status:** fixed. Two-slot managed runtime workers receive idSlot=1 and send id_slot. Single-slot requests and explicit external loopback providers remain unpinned.
- **Changed locations:** [src-tauri/src/web_search.rs:119](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:119>); [src/web/models/local_provider.ts:78](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/models/local_provider.ts:78>).
- **Maintained regression:** [src/web/models/local_provider.test.ts:28](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/models/local_provider.test.ts:28>) — `sends the configured worker slot and leaves single-slot requests unpinned`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Provider payload behavior is tested. Actual slot placement and cache behavior are not measured on a live llama.cpp process.

## B1 — Canonical local inference URL validation

- **Status:** fixed. Rust and worker providers accept only credential-free loopback HTTP(S). Canonical hostname/IP parsing accepts ::1, 127/8, uppercase localhost and its trailing dot, and rejects spoofed suffixes, user-info tricks, mapped IPv6, and other schemes.
- **Changed locations:** [src-tauri/src/providers.rs:241](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/providers.rs:241>); [src/web/models/local_provider.ts:36](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/models/local_provider.ts:36>).
- **Maintained regression:** [src-tauri/src/providers.rs:462](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/providers.rs:462>) — `loopback_urls_require_http_without_credentials_and_accept_ipv6`; [src/web/models/local_provider.test.ts:22](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/models/local_provider.test.ts:22>) — `rejects credential-bearing and non-loopback inference URLs`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** DNS for localhost is trusted as a local machine service name. Public-page URLs follow a separate stricter pinned transport policy.

## B2 — Google errors can disclose credentials

- **Status:** fixed. HTTP failures expose status only; transport diagnostics redact both raw and URL-encoded Google keys before reaching traces or tool output.
- **Changed locations:** [src/web/search/google_provider.ts:43](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/google_provider.ts:43>); [src/web/search/google_provider.ts:77](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/google_provider.ts:77>).
- **Maintained regression:** [src/web/search/google_provider.test.ts:5](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/search/google_provider.test.ts:5>) — `never exposes the API key from HTTP bodies or transport errors`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** This fixes demonstrated error leakage. It does not establish a universal secret-redaction guarantee for every third-party error or user-written log.

## B3 — Model verifier can bless invented claims

- **Status:** fixed. A model SUPPORTED decision must also pass the deterministic verifier against its cited evidence subset; a real source ID alone cannot certify an invented claim.
- **Changed locations:** [src/web/verification/claim_verifier.ts:174](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/verification/claim_verifier.ts:174>).
- **Maintained regression:** [src/web/verification/claim_verifier.test.ts:192](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/verification/claim_verifier.test.ts:192>) — `model cannot certify an invented claim using a real source ID`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Lexical verification conservatively rejects some paraphrases. The old model-paraphrase expectation was updated; model-assessed support no longer upgrades it to independently verified support.

## B4 — Unsanitized stored and raw web tool results

- **Status:** fixed. Worker JSON serialization sanitizes every returned string, including stored open/find passages, raw fetch text, titles/headings/links and search results. Existing extraction-time sanitization remains.
- **Changed locations:** [scripts/web-worker.mjs:197](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/web-worker.mjs:197>).
- **Maintained regression:** [scripts/worker-input.test.mjs:25](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/worker-input.test.mjs:25>) — `stored open and find passages neutralize web directives`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** Directive neutralization is pattern-based, not a proof of injection immunity. Tool permission and offered-tool checks remain the authority boundary.

## B5 — Total research deadline discards partial work

- **Status:** fixed. Rust sends an absolute deadline with a 15-second return reserve. Staged search/model/fetch work is raced against it; already committed sources/facts survive later stalls. The worker drains the partial response and exits after closing SQLite so pending transports cannot hold it past the outer kill.
- **Changed locations:** [src-tauri/src/web_search.rs:83](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:83>); [src/web/staged_research.ts:268](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/staged_research.ts:268>); [src/web/index.ts:366](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/index.ts:366>); [scripts/web-worker.mjs:200](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/scripts/web-worker.mjs:200>).
- **Maintained regression:** [src/web/staged_research.test.ts:10](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/staged_research.test.ts:10>) — `returns collected evidence when a later query exceeds the total deadline`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** The stalled-query test failed before, passed after, and was then converted to fake timers after a load-sensitive 100 ms fixture failure. This deadline preservation is staged research; no-model non-staged research and individual fetch actions still retain their existing outer process bounds.

## B6 — Worker inference queue and exact input sizing

- **Status:** fixed. Generation calls serialize per provider, so one-slot AREX batches do not concurrently consume their per-request generation timers. Authenticated chat/completions/input_tokens measures the exact request; fallback uses tokenizer/UTF-8 bytes plus a 64-token template reserve. Generation observes the total deadline.
- **Changed locations:** [src/web/models/local_provider.ts:57](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/models/local_provider.ts:57>); [src/web/models/local_provider.ts:93](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/models/local_provider.ts:93>).
- **Maintained regression:** [src/web/models/local_provider.test.ts:6](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src/web/models/local_provider.test.ts:6>) — `uses exact chat input tokens and serializes calls for one inference slot`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** The combined regression was red due to incorrect input budgeting before the fix; it now verifies exact-count selection and maximum concurrency=1. The prior parallel timer start was also established by code trace. Serial execution trades throughput for bounded single-slot reliability.

## B7 — AREX controls bypass offered-tool checks

- **Status:** fixed. Harness offered-tool validation now precedes AREX search/visit/update_context/finish dispatch, covering direct and child dispatch paths.
- **Changed locations:** [src-tauri/src/harness.rs:346](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/harness.rs:346>).
- **Maintained regression:** [src-tauri/src/harness.rs:1028](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/harness.rs:1028>) — `arex_controls_cannot_execute_when_not_offered`. Pre-fix evidence: code trace; post-fix test green.
- **Remaining risk / follow-up:** Clear code trace demonstrated the bypass. The first native regression invocation was blocked before test entry by B9; therefore no pre-fix assertion failure is claimed. After the loader fix the regression passes.

## B8 — AREX cited evidence enables Markdown/HTML

- **Status:** fixed. finish_answer escapes HTML and Markdown syntax in evidence text while application code constructs validated source links.
- **Changed locations:** [src-tauri/src/arex.rs:220](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/arex.rs:220>).
- **Maintained regression:** [src-tauri/src/arex.rs:284](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/arex.rs:284>) — `arex_finish_renders_evidence_and_does_not_invent_citations`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** The existing regression was extended with injected image/script evidence and observed red/green. The answer remains normal model-authored Markdown; this change is limited to the supporting evidence field.

## B9 — Windows native test binary lacks Common Controls manifest

- **Status:** fixed. Executing the harness regression pulled in TaskDialogIndirect; the native test executable failed at loader startup with STATUS_ENTRYPOINT_NOT_FOUND. build.rs supplies the Common Controls v6 manifest dependency for Windows link targets.
- **Changed locations:** [src-tauri/build.rs:5](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/build.rs:5>).
- **Maintained regression:** [src-tauri/src/harness.rs:1028](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/harness.rs:1028>) — `arex_controls_cannot_execute_when_not_offered`. Pre-fix executable loader failure → full suite green.
- **Remaining risk / follow-up:** Red observation was process startup failure, not a Rust assertion. The full native suite and clippy run pass after the linker change. The loader diagnostic is local and ignored; no registry or user settings were changed.

## B10 — Children broaden parent tool access

- **Status:** fixed. Snapshots capture the actual offered aliases. Background reconstruction intersects with them; foreground children inherit only the parent catalog. Child harness snapshots carry the narrowed catalog onward so nested children/workflows cannot regain removed tools.
- **Changed locations:** [src-tauri/src/subagents.rs:215](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/subagents.rs:215>); [src-tauri/src/subagents.rs:559](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/subagents.rs:559>); [src-tauri/src/subagents.rs:757](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/subagents.rs:757>); [src-tauri/src/subagents.rs:881](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/subagents.rs:881>); [src-tauri/src/chat.rs:1743](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/chat.rs:1743>).
- **Maintained regression:** [src-tauri/src/web_search.rs:333](<C:/Users/Papan Ghosh/Desktop/Projects/LocalLM/src-tauri/src/web_search.rs:333>) — `keyless_loopback_snapshot_succeeds_but_remote_and_keyless_claude_fail`. Red → green, subject to secondary-test qualifications below.
- **Remaining risk / follow-up:** The snapshot regression was extended and failed with “Children must not acquire unoffered tools”; it now checks both empty and explicitly offered catalogs. Workflow/ralph/recursive narrowing is additionally established by shared-call code trace, not live model execution.

## Related audit coverage and concerns left unchanged

- **Policy:** reviewed worker, native reqwest/browser, child dispatch, PTC, workflow and ralph paths. PTC resolves actual inherited tools and applies approval checks; children preserve approval pins and now cannot broaden offered catalogs. Browser retrieval checks offline and Browser settings. Explicit retrieval is blocked offline; shell and unknown-network MCP retain their own permissions. `web_search_disabled` gates the research worker, while independently offered native browser/weather capabilities have their own policy. No global network sandbox was inferred.
- **Public fetch SSRF:** existing transport resolves DNS and pins the checked address to the socket; every redirect is validated. Existing tests cover private/mapped/decimal/octal IP forms, credential-bearing URLs, schemes and redirect rejection. Renderer resources use that transport, a dead proxy, blocked WebSockets/service workers and bounded output. The TypeScript preliminary DNS guard alone is insufficient, but the actual public-page transport fails closed; no demonstrated bypass was found. Configured SearXNG is a trusted service endpoint and is intentionally distinct from public-page SSRF rules.
- **Grounding:** EvidenceExtractor already grounds model output by normalized retrieved-text membership; source IDs/URLs are application-owned. Non-staged AnswerGenerator uses the verifier, now hardened by B3. No additional reproducible evidence-promotion defect was found. Lexical grounding cannot prove semantic truth or source correctness.
- **Secrets:** audited worker configuration, diagnostics, session/trace paths and managed runtime headers. Demonstrated Google error leakage is fixed by B2 and native UI exposure by A20. User-authored `web-search.json` overrides and CLI configuration may still contain secrets; they are not silently rewritten. Runtime/third-party logs cannot be universally proven secret-free without live service testing. Arbitrary server errors containing transformed secrets remain a speculative concern.
- **Defaults:** reviewed scalar setting reads for missing-value behavior. Other false/zero defaults are intentional, or callers already use optional defaults/sentinel handling; no additional reproduced A19 pattern was changed.
- **Process bounds:** existing concurrent pipe drains cap stdout/stderr and cancellation uses KillOnDrop plus a Windows Job Object (Unix process group). Existing process-tree tests passed. Browser-process kill behavior was not independently measured with a live rendered page during this task. Hashed installed worker generations may remain on disk; deleting prior generations could race active calls, so no cleanup was added. Temporary installation files are removed on rename failure.
- **Input size:** worker stdin is byte bounded; AREX audit fields are byte truncated. Extremely large trusted overrides or heavily JSON-escaped evidence can exceed total wire size and fail safely; automatic semantic truncation was not introduced.
- **Deadlines:** partial preservation covers staged research. Individual tool calls and no-model research remain bounded by existing outer deadlines; a separate generic partial-results contract was not invented.
- **Documentation:** updated README.md, docs/MODELS.md, docs/AREX.md and docs/WEB_SEARCH_PHASE1.md for per-slot context, runtime measurement, offline scope, key vault/clear/migration, quota/fallback/provenance, extraction limits, stored passages, renderer mediation and token budgeting. Earlier dated live-test statements in existing docs describe earlier work, not a new live test here.

## Full verification

All required commands completed successfully on the final implementation.

| Command | Final output summary |
| --- | --- |
| `npm run typecheck` | Exit 0; app and e2e TypeScript checks pass |
| `npm run lint` | Exit 0; oxlint with deny-warnings |
| `npm test` | Exit 0; 76 files passed, 1 skipped; 445 tests passed, 1 skipped; 50.88 s |
| `cargo test --locked --manifest-path src-tauri/Cargo.toml` | Exit 0; 394 passed, 2 ignored, 0 failed; main/doc-test targets also pass; tests 63.55 s |
| `cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | Exit 0; strict all-target check finished in 41.84 s |
| `node --test scripts/*.test.mjs` | Exit 0; 39 passed, 0 failed; 8.59 s |
| `npx playwright test` | Exit 0; 10 passed in Edge against Vite :1420; 51.0 s |
| `npm run build:web` plus SHA-256 before/fresh rebuild | Exit 0; hashes identical |

Embedded worker SHA-256: `8A1FE47368EA46D6314233814F03D42F699265BABB34895FC3AACC769B03E386`.

Logs: `.local/web-runtime-fixes-20260930/{typecheck,lint,vitest,cargo-test,clippy,worker,playwright,old-repros,old-worker-repro}.log`. Earlier full-suite runs under concurrent native compilation hit existing 5-second UI test timeouts and a 100 ms deadline fixture assumption; the owned deadline fixture now uses fake timers. The final ordinary `npm test` run passed without worker-count or timeout overrides. Skipped/ignored coverage remains visible rather than being counted as passed. Vitest prints the existing jsdom canvas warning; it does not fail the suite.

## Untouched defect-asserting reproductions

`npx vitest run --config .local/web-runtime-audit-20260930/vitest.config.mjs` exits 1 with **12/12 expected failures**:

| Old assertion | Fixed behavior |
| --- | --- |
| AREX invented number gets real URL | Invented line rejected (A4) |
| Reversed synthesis negation accepted | Attributed fallback (A5) |
| All engines unresponsive called working | Search/health failure (A12) |
| Google health ready without request | configured_not_tested (A14) |
| Google batch exceeds quota | Atomic strict cap (A15) |
| Staged fallback trace says SearXNG | Actual Google provenance (A17) |
| Prose failure discards table | Table facts retained (A7) |
| SearXNG prefix discarded | Prefix preserved (A18) |
| Same-domain pages become one | Two URLs allowed (A11) |
| Query syntax removed | Operators/quotes preserved (A9) |
| Google ignores freshness/page | dateRestrict/start applied (A16) |
| AREX fact beyond prefix lost | Whole-page ranking (A10) |

`node --test .local/web-runtime-audit-20260930/worker-repro.test.mjs` exits 1 with **two expected failures and one pass**. UTF-8 corruption fails because 咖啡 arrives intact (A6); combined JSON-array query fails because two decoded queries arrive (A8). The cooldown assertion still passes because the repro helper explicitly forces `databasePath: ':memory:'` for every new process. This does not exercise A13's shared-database prerequisite; the maintained shared-storage regression passes. The old reproductions were not edited.

## Live testing boundary

No real model-plus-SearXNG end-to-end test was performed. A managed llama-server executable and GGUF were present, but SearXNG did not listen at 127.0.0.1:8080 and Docker CLI was unavailable. No running reverse proxy or two-slot managed runtime was used. Offline/tool boundaries were tested with temporary stores, provider/HTTP fixtures and mocks; Edge e2e tests cover the UI. Real known-answer retrieval, /searxng/ proxy behavior and measured two-slot /props remain follow-ups when those services are available.
