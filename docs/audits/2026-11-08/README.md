# Audit evidence — 8 November 2026

The audit and plan cover the working tree at `609209e` plus its existing uncommitted changes. Production application files were not edited. Source inventory records hashes of the original app/test/script files, not the disposable probe additions.

- [Audit](/home/papan/Desktop/Projects/localLM/docs/FULL_CODEBASE_AUDIT_2026-11-08.md)
- [Repair and response design plan](/home/papan/Desktop/Projects/localLM/docs/INFERENCE_AND_RESPONSE_FIX_PLAN_2026-11-08.md)
- [Source inventory](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/source-inventory.csv)

## Results

| Evidence | Meaning |
| --- | --- |
| [Frontend suite](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/frontend-suite.txt) | 894 passed, 1 skipped |
| [Typecheck](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/typecheck-result.txt) | Fails at existing `Chat.tsx:42`; transcript of the observed initial command error |
| [Rust check](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/rust-check.txt) | Passed |
| [First Rust suite](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/rust-suite-first-run.txt) | 1,049 passed, 1 cancellation timing failure, 35 ignored |
| [Isolated cancellation](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/cancellation-isolated.txt) | Passed in 0.60 s |
| [Full Rust rerun](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/rust-suite-rerun.txt) | 1,050 passed, 0 failed, 35 ignored; `--test-threads=2` |
| [Rust diagnostics](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/rust-probe-results.txt) | 11 diagnostic reproductions passed |
| [Frontend diagnostics/control](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/frontend-probe-results.txt) | 9 diagnostic reproductions and 1 interaction control passed |

**Diagnostic probes intentionally assert the defective behavior.** Their passing results mean problems reproduced. They are not fixes or correctness gates. The implementation plan requires production regressions with the expectations reversed to assert the repaired behavior.

## Reproduce

From `/home/papan/Desktop/Projects/localLM`:

```bash
python3 docs/audits/2026-11-08/run-rust-probes.py
npx vitest run --config docs/audits/2026-11-08/frontend-probes.config.ts --reporter=verbose
```

The Rust runner copies `src-tauri`, `src` and `catalog` into a new directory under `.local/qa/audit-2026-11-08/`, excluding build/dependency directories. It appends the three retained `.inc` probe modules to that disposable source copy. It reuses the Cargo target directory for dependencies and runs only `audit_20261108` tests. It never patches the original application source or opens its database. New temporary copies are retained so they are inspectable.

The frontend configuration imports the production components directly; `app-probes.test.tsx` reuses the existing App fixture setup. Its APIs, artifact reads and stream events are mocked. These tests are outside the normal `src/**/*.test.*` suite. The Rust stream probes use scripted local HTTP servers and temporary workspaces, without real models, credentials or windows. A socket-restricted host must report those fixtures unavailable rather than passing.

The response proposal was checked with jsdom for default state, five scenario states, allow/deny simulation and failure expansion. No native/browser visual inspection was performed. OpenCode reference source was downloaded read-only under `.local/qa/audit-2026-11-08/opencode-reference/` at commit `ece56ca58419083275f2a2a81ac28d05f4a1b89b`; no reference application was installed or launched.

No app browser preview, paid inference or production data mutation was part of this review. The source inventory, fixture assertions, earlier audit reconciliation and current check results are the boundaries of the evidence.
