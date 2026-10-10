# LocalLM roadmap — implementation spec and checklist

Started 2026-10-09 on top of a checkout with substantial uncommitted user work (see `docs/ROADMAP_BASELINE.md`). That work is preserved; nothing is reset, stashed, or auto-committed.

States: `pending` · `in-progress` · `verified` (automated checks) · `native-verified` (desktop app) · `blocked` (reason given).

## Reused systems (extend, do not replace)
- Turn loop, plan mode, budgets: `chat.rs`, `continuation.rs`, `agent_run.rs` (run records, run events, recovery ledger).
- Harness registry and preset tools: `harness.rs`, `presets.rs`, `capabilities.rs`, `approval.rs`.
- Workspace edits with fresh-hash conflict checks: `workspace.rs`, `workspace_ui.rs`; review inspector `WorkspacePanel.tsx`.
- Memory facts and repo survey: `memory.rs`; artifacts: `artifacts.rs`; research engine and verification: `src/web/**`; oracles: `scripts/eval-oracles.mjs`.
- Single model operation lock: `AppState.operation` (`try_lock` in `send_message`) — the basis of inference ownership.

## Contracts introduced
- **Interaction mode** (`ask | plan | act`) is per conversation, separate from permission mode (`ask | auto-reads | full`). Enforced in the backend tool filter, not the prompt. Legacy `plan_mode: bool` maps to `plan` only for that turn; stored conversations without a mode default to `act` with their existing permission mode unchanged (never broader).
- **Effective capabilities**: one backend command returns every tool with `{available, enabled, permittedInMode, requiresApproval, blockedReason}`; the Tools summary renders only from it.
- **Research budget**: one `ResearchBudget` object (initial queries, follow-up queries, pages, retries, time) owned by `src/web/config`, used by planner, staged research and the UI copy.

## Milestones
| # | Milestone | State |
|---|---|---|
| 0 | Baseline + this spec | verified |
| 1.1 | Minimum-window sidebar overlay | verified |
| 1.2 | Segmented control keyboard exception | verified |
| 1.3 | UTF-8 safe repo-survey truncation | verified |
| 1.4 | Effective tool availability view | verified |
| 1.5 | Ask / Plan / Act enforced in backend | verified |
| 1.6 | Research budget contract | verified |
| 1.7 | Duplicate status text, tab/radio semantics | verified |
| 2 | Task worktrees, checkpoints, review/apply/discard | verified |
| 3 | Evidence-backed completion | verified |
| 4 | Project intelligence and visible memory | verified |
| 5 | Task inbox, queue, steering, recovery | verified |
| 6 | Model qualification and guided setup | verified |
| 7 | Enforced execution and network isolation | verified |
| 8 | Recipes and scheduled automation (requires 7) | verified |
| 9 | Private knowledge workspace | verified |
| 10 | Deliverable workspace | pending |
| 15 | Cleanup and consolidation | pending |
| 16 | Good-to-have additions | pending |

## Ordering
Milestone 1 ships first (defects and truthful controls). 2–5 are the core (review, evidence, memory, task lifecycle). 8's write-capable scheduling is gated on 7 and stays disabled until an enforced boundary is tested. 9, 10, 15, 16 follow.

## Verification honesty
Mocked Playwright coverage is labeled as such. Native desktop verification is recorded separately per feature; features without it are not reported as native-verified.
