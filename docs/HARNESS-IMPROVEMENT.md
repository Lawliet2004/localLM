# LocalLM Harness Improvement — Implementation Checklist

Primary goal: reliable code execution, key-free web research, valid structured
results, and recoverable long-running tasks around the default local model
(`BAAI_AREX-Turbo-Q4_K_M.gguf`). Harness only; no weight changes; prompts
alone never guarantee correctness.

Constraints: SearXNG default, no paid search APIs or mandatory keys; local
AREX path works without remote credentials; optional GPT/Claude only through
explicitly configured connections, never silent remote use; preserve
uncommitted changes, settings, conversations, permissions, credentials; reuse
Rust/Tauri + TypeScript + React + SQLite + worker architecture; small changes.

## Baseline (2026-09-17, before changes)

- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: 233 tests running
  (suite started; full run exceeds 30 s in this environment, same as before).
  Prior recorded state: full suite green per docs/PROGRESS.md.
- `npm run build`: PASS (tsc + vite, 603 kB bundle, chunk-size warning only).
- `node scripts/web-transport.test.mjs`: 3/3 PASS.
- `npm test` (vitest): suite starts green; full run exceeds 30 s here, so
  verification uses targeted file runs plus build.
- SearXNG `http://127.0.0.1:8080`: UNREACHABLE (connection refused).
  Live search verification blocked; fixture tests only.
- Default model weights `BAAI_AREX-Turbo-Q4_K_M.gguf`: NOT present.
  `.local/models` holds MiniCPM5-2B.Q6_K.gguf only. Live AREX tool-call
  verification blocked; deterministic protocol fixtures only.
- Docker: NOT installed. Container isolation tests blocked.
- Representative task baselines (pre-change behavior, from code inspection):
  - Code execution returns `{provider, exitCode, stdout, stderr, error,
    isError, durationMs}` with no separate structured-result channel; logs
    and results share stdout; no schema validation; truncation surfaces as a
    generic error string without an explicit truncated flag.
  - `web_open`/`web_find` (scripts/web-worker.mjs) read only previously
    compressed evidence passages, not full stored document text; PDFs and
    scanned documents have no path; no page/section/passage ranges.
  - Search has no `pageno` pagination, no per-engine diagnostics, unbounded
    query fan-out concurrency, single-attempt fetch retry policy only in the
    transport layer.
  - Evaluator returns `citationCorrectnessRate = 1.0` when zero citations
    exist, so empty answers can score 100%.
  - Compaction archives verbatim history but carries no structured research
    state (requirements, findings, source refs).

Timing/failure notes are per-section below. No measured improvement is
claimed without a before/after rerun of the same task.

## Stages

- [x] 0. Audit, baseline, contracts doc (this file)
- [x] 1. Versioned structured tool contracts (`src-tauri/src/tool_envelope.rs`)
- [x] 2. Structured code execution (result channel, schemas, backgrounds)
- [x] 3. SearXNG reliability + diagnostics (pagination, budgets, Retry-After)
- [x] 4. Full-document reading + PDF support (pages, ranges, OCR failure)
- [x] 5. Iterative research + evidence state
- [x] 6. Checkpointing, resume, compaction upgrade
- [x] 7. Provider adapters (AREX verification, optional Claude Messages API)
- [x] 8. Verification, structured final output, UI presentation
- [x] 9. Acceptance tests A–E, full checks, final report (below)

## Acceptance map

- A (structured execution): Rust `execution` + `tool_envelope` tests.
- B (search/reading): `src/web` fixture tests + `web-transport` tests.
- C (research reliability): engine + evaluator + injection tests.
- D (long-running): `research_tasks` + compaction + cancellation tests.
- E (provider compat): deterministic adapter tests; live tests gated on
  credentials/authorization and reported as blocked when absent.

## After results (2026-09-17, same machine)

- `cargo test --lib -- execution tool_envelope artifacts chat::finish_tests`:
  29/29 PASS, including nested/None/Unicode serialization, log/result
  separation, invalid-JSON/missing-result/unsupported-value/truncation
  failures, nonzero-exit + timeout reporting.
- `cargo test --lib -- tool_calls claude_adapter research_tasks compaction
  harness::tests inference::tests providers::tests
  store::tests::research_tasks`: 38/39 PASS after a one-line test fix
  (`{"q":}` is balanced structural JSON and is not truncation); rerun green
  for the fixed file. Full lib suite: pre-existing size (~250 tests) exceeds
  the 30 s tool window per binary run; affected modules all green, strict
  Clippy green.
- `cargo clippy --all-targets -- -D warnings`: PASS.
- `npm run build` (tsc + vite + worker bundle): PASS.
- `node scripts/web-transport.test.mjs`: 3/3 PASS (SSRF, redirect, robots).
- Targeted vitest acceptance set (10 files, 84 tests): ALL PASS —
  SearXNG reliability (8), document reading (5), research state (7),
  verification acceptance (6), engine regressions (11+7+3+1),
  CommandRunCard incl. Result tab (11), Chat (25).
- Representative before/after on the same tasks:
  - Execution: before, `print('hi')` with no result channel returned
    `isError: false` success with stdout only; after, the same code returns
    `status: failure / missing-result` (logs are not results), while
    `locallm_result({...})` returns `status: success` with `data.result`
    validated and `data.logs` separate.
  - Search: before, one failed query failed the batch silently and page-two
    hits were unreachable; after, healthy results are kept with per-query
    failures, `pageno=2` recovery is budgeted, and engine diagnostics persist.
  - Reading: before, `web_open` returned only compressed evidence; after, it
    returns full-text passages with stable refs plus headings/links, PDFs
    carry page refs, scanned PDFs fail as `OCR required`.
  - Evaluation: before, zero citations scored 1.0; after, zero citations
    score 0 and citation-requiring categories cannot pass without citations.

## What remains unverified (blocked, not skipped)

- Live SearXNG (`docker compose up searxng` then `npm run research`):
  daemon unreachable here; fixture tests pass, live suite not run.
- Live AREX tool-use + structured-result smoke (`BAAI_AREX-Turbo-Q4_K_M.gguf`
  loaded, approval-driven `run_code`): weights absent here (only MiniCPM5
  present); deterministic protocol tests pass, live smoke not run.
- Live GPT/Claude: no credentials configured here by design; deterministic
  adapter tests pass; live remote tests run only with explicit authorization.
- Browser rendering fallback for JS-dependent pages: intentionally not
  bundled; such pages surface as snippet-only evidence with an explicit
  limitation string.
- Full `npm test` / full `cargo test` in one invocation: exceed the 30 s
  per-command window here; verified via targeted module runs + build +
  clippy, all green.

