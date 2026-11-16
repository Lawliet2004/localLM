# Harness audit evidence — 8 November 2026

Main report: [LocalLM harness reliability audit](/home/papan/Desktop/Projects/localLM/docs/HARNESS_RELIABILITY_AUDIT_2026-11-08.md).

This is an audit of commit `609209e7c5c3b69a456dca577a7ee6bcc046a0e7` plus its existing uncommitted work. The original application source was not edited. [Source integrity](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/source-integrity.json) verifies hashes against [the initial inventory](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/source-inventory.json).

The full report distinguishes passing correctness suites, reproduced defects, imposed limits, upstream dependency advisories and unperformed live acceptance. The unchanged ignored integration fixture has a repeatable failure; the default Rust suite also had one timing-sensitive initial failure. Reduced-concurrency Rust results do not erase either observation.

**Reproduce the focused diagnostics**

From `/home/papan/Desktop/Projects/localLM`:

```bash
python3 docs/audits/harness-2026-11-08/run-probes.py > docs/audits/harness-2026-11-08/probes.txt 2>&1
```

The runner copies `src-tauri`, `src`, `catalog` and package manifests to a new directory under `.local/qa/harness-audit-2026-11-08/`, excluding target/dependency/git directories. It appends the retained `.inc` modules only to that copy. It uses temporary workspaces/databases and scripted loopback servers. It never opens the real application database or starts a real model, browser or provider session.

It reuses `src-tauri/target` for dependency compilation; do not run other Cargo builds concurrently with it. Its disposable source directory is retained and recorded in `probe-source-location.txt`. The snapshot must include the existing working-tree changes; a clean checkout of HEAD is not equivalent.

The final result was **15 probes passed**. Fourteen are diagnostic assertions of defects or limits; one is a positive acknowledged-process-stop control. Passing diagnostics do not certify correct behavior. Unix process-exit checks are platform-gated, so another platform may execute fewer probes. The process fixture's descendants exit promptly; background child fixtures are explicitly interrupted.

The runner also changes exactly one pre-existing test fixture in the disposable copy: it supplies a third scripted model response while keeping all original assertions. The [retained diff](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/integration-fixture-adjustment.diff) makes that diagnostic adjustment explicit. The additional response services the verification reminder after a post-tool hook. The [control passed](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/loopback-fixture-control.txt); this does not change the [failed original fixture](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/loopback-integration-serial.txt).

Structured probe observations are in [probe-facts.json](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/probe-facts.json). The PTC comparison uses a scripted provider and proves observed orchestration/result parity only. It does not measure a real model's plan selection, task accuracy or live inference-cache hit rate.

The retained `.txt` outputs are portable evidence. Additional ignored `.log` files contain execution transcripts and intermediate probe refinements. Initial refinement failures reflect corrected fixture assumptions: Linux kills the direct child, PTC needs the Execution source enabled, and the budget warning's wire text differs from its fallback text. They are not additional application findings.

**Additional checks**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=2
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --ignored --test-threads=1 chat:: api_tools:: subagents::
cargo check --manifest-path src-tauri/Cargo.toml --lib
npx vitest run
npm run typecheck
npm run build
node --import ./scripts/register-test-loader.mjs --test scripts/web-transport.test.mjs scripts/web-render.test.mjs scripts/web-storage.test.mjs scripts/searxng-worker.test.mjs scripts/weather-worker.test.mjs
python3 scripts/benchmark-response-latency.test.py
npm audit --omit=dev --json
npm audit --json
```

`npm audit` returned nonzero because of flagged vulnerabilities, not because its network request failed. Dependency audits change with published advisory data; preserve the saved JSON when comparing results. No dependency upgrade was applied. No RustSec scan, live model/provider benchmark, browser preview, actual hour-long soak, or multi-day run is included.

Toolchain at verification: Rust/Cargo 1.99.0, Node 22.22.1, npm 9.2.0, Linux x86_64. The report and evidence use the user's date, 8 November 2026.
