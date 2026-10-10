# Baseline Check Results

Date: 2026-10-09

Working tree state at time of run: `git status --short | wc -l` = **342** entries (pre-existing uncommitted changes; nothing was modified by these checks).

Playwright e2e was intentionally skipped.

| Check | Command | Result |
|-------|---------|--------|
| Typecheck | `npm run typecheck` | **PASS** |
| Lint | `npm run lint` | **PASS** |
| Unit tests | `npm test` | **PASS** |
| Script tests | `npm run test:scripts` | **PASS** |
| Rust lib tests | `cargo test --manifest-path src-tauri/Cargo.toml --lib` | **PASS** |
| Clippy | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | **PASS** |
| Rust fmt | `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | **FAIL** |

## Details

### `npm run typecheck` — PASS
`tsc --noEmit && tsc --noEmit -p tsconfig.e2e.json` — clean, exit 0.

### `npm run lint` — PASS
oxlint `src` + `e2e` with `--deny-warnings`: 0 warnings, 0 errors, 218 files, 108 rules.

### `npm test` — PASS
Vitest run: **554 passed, 1 skipped** (555 total) across **89 passed / 1 skipped** test files. Duration ~29.6s. No failures.

### `npm run test:scripts` — PASS
`node --test scripts/*.test.mjs`: **47 pass, 0 fail**, 0 skipped. Duration ~6.4s.
(One benign Node `ExperimentalWarning` for the SQLite feature.)

### `cargo test --lib` — PASS
**454 passed, 0 failed, 2 ignored**. Duration ~62.7s.
Note: `connectors::tests::malformed_protocol_input_and_unexpected_exit_are_reported` ran >60s but passed.

### `cargo clippy --all-targets -- -D warnings` — PASS
Finished dev profile in ~2.5s, no warnings or errors emitted.

### `cargo fmt -- --check` — FAIL (exit 1)
Formatting diffs detected: **1,084 hunks across 53 files** under `src-tauri/src/`. The tree is broadly unformatted relative to rustfmt defaults (likely due to the large set of uncommitted changes).

Top offenders by hunk count:

| File | Hunks |
|------|-------|
| `chat.rs` | 137 |
| `store.rs` | 99 |
| `subagents.rs` | 78 |
| `harness.rs` | 75 |
| `sandbox.rs` | 42 |
| `execution.rs` | 37 |
| `inference.rs` | 34 |
| `workspace_ui.rs` | 33 |
| `workspace.rs` | 33 |
| `arex.rs` | 33 |
| `web_search.rs` | 31 |
| `compaction.rs` | 31 |
| `commands.rs` | 30 |
| `connectors.rs` | 27 |

Plus ~30 more files with 1–22 hunks each (down to single-hunk files like `tool_compat.rs`, `hardware.rs`, `model_catalog.rs`).
