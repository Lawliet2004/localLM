# Goal: Harness reliability: long autonomous runs complete

- Status: active
- Created: 2026-11-09
- Updated: 2026-11-09

## Objective

LocalLM agent loop runs long autonomous tasks to completion without harness-caused mid-run stops. The stopped runs are root-caused, the loop is compared against other harnesses, and the continuation, tool-alignment, timeout and prompt defects behind the stops are fixed and verified.

## Success criteria

- [x] Root-cause report identifies why the 2,652 s soak run and the Omen round-limit run stopped, with each cause mapped to a file:line and labelled harness or model
- [ ] Side-by-side comparison of LocalLM and Claude Code (plus any other harness reviewed) covers loop continuation, tool-call alignment, timeout handling, prompt/context size and long-run durability, with a prioritized fix list
- [ ] A regression test exhausts a small round/time/search budget and proves the task continues into another segment instead of ending
- [ ] Model-issued tool calls that do not match the advertised schema are rejected or corrected before dispatch, with regression coverage
- [ ] A fixture with a slow or timing-out stream continues the run instead of ending it; model and harness timeouts are distinguished
- [ ] System prompt is compact, versioned and stable across rounds; tool definitions stay byte-stable; a prefix-stability test passes
- [ ] cargo test --lib, cargo check --lib, npx vitest run and npm run typecheck pass after the fixes

## Non-goals

- Real multi-day unattended runs on real accounts (needs real hardware and time)
- Model-quality fixes (MiniCPM/DeepSeek accuracy); harness-side causes only
- Rewriting the harness or adding new tools/features beyond the loop fixes
- Native Windows/macOS sign-off and packaged installer validation
- Restoring the removed homepage response-mode selector

## Constraints

- Do not break existing tests (AGENTS.md baseline; current counts in docs/HARNESS_ACCEPTANCE_REVALIDATION_2026-11-09.md)
- Keep the locallm.tool-envelope/1 contract; normalize once at model boundaries
- Respect AGENTS.md: removed homepage selector ban; preserve uncommitted work; no reset/stash/clean
- Ponytail full: smallest change that fully works; no new dependency for a few lines
- Sandbox: TCP socket creation fails; real HTTP/socket fixtures must run on a permitted host and be reported as not executed here otherwise

## Milestones

### M1 — M1: Diagnose the stopped runs

- Status: complete

Acceptance:

- [x] Root-cause report for the 2,652 s soak stop and the Omen round-limit stop, each cause mapped to file:line and labelled harness or model
- [x] Evidence covers the loop exit paths, timeout paths and budget-exhaustion paths

### M2 — M2: Compare harness loop design with other harnesses

- Status: pending

Acceptance:

- [ ] Comparison doc covers continuation policy, tool-call alignment, timeout handling, prompt/context size and long-run durability for LocalLM vs Claude Code and any other harness reviewed
- [ ] Prioritized fix list with severity and code locations

### M3 — M3: Durable continuation across budgets

- Status: pending

Acceptance:

- [ ] The loop crosses a round/time/search budget boundary and continues the task into another segment instead of ending
- [ ] Regression test exhausts a small budget and proves continuation; a 1 h soak still completes

### M4 — M4: Tool-call alignment and timeout handling

- Status: pending

Acceptance:

- [ ] Model-issued calls that do not match the advertised schema are rejected or corrected before dispatch
- [ ] A slow or timing-out stream continues the run; model and harness timeouts are distinguished
- [ ] Regression tests for both

### M5 — M5: System prompt quality and long-run durability

- Status: pending

Acceptance:

- [ ] System prompt is compact, versioned and stable across rounds; tool definitions byte-stable
- [ ] Auth token refresh, context-overflow and process-lifecycle paths are hardened for long runs
- [ ] Prefix-stability test and full gates pass

### M6 — M6: Final verification

- Status: pending

Acceptance:

- [ ] cargo test --lib, cargo check --lib, npx vitest run and npm run typecheck pass
- [ ] A representative long run completes end to end with evidence

## Current focus

M2: Write the continuation comparison against the confirmed 64-round stop (docs/HARNESS_STOP_ROOT_CAUSE_2026-11-09.md) and the existing Claude Code evidence. Do not start the segment-continuation fix until that list is written.

## Decisions

- 2026-11-09 — Goal created. Replaced the stale 2026-11-04 tracker (crash/plugin/CSP/tests/big files) at the user's choice; that work is recorded below as a follow-up.

## Log

- 2026-11-09 — Tracker created.
- 2026-11-09 — Follow-up (not this goal): the previous tracker's items — responses_adapter crash safety, plugin wiring, CSP without unsafe-inline, tests for risky files, and splitting store.rs/chat.rs/workspace_patch.rs/App.tsx — remain open. Revisit after this goal or if the user asks.
- 2026-11-09 — M1 complete. Latest Omen goal turn `5c74c45b` paused `budget_exhausted` after 64 rounds / 381 s; task clock was 1800 s. Cause is the hard loop cap in `chat.rs:1958` with no next segment. 2652 s is a soak heartbeat (`harness_endurance_tests.rs:135`), not a retained harness exit. Report: `docs/HARNESS_STOP_ROOT_CAUSE_2026-11-09.md`. Gates not re-run; this step changed no product code.
- 2026-11-09 — Fixed the four harness bugs from that report, not the 64-round cap. Over-limit shell timeouts and wait fields are clamped instead of rejected; `budget_exhausted` no longer counts tool operations as rounds; a known tool failure is `Failed`, not `Unknown`. `cargo test --lib`: 1117 passed, 1 failed (old yield rejection). That assertion was updated; the focused rerun passed. Full suite not repeated after the test-only edit.
