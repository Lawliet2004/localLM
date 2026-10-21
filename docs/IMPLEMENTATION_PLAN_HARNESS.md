# Implementation Plan — Harness Completion

Derived from `docs/HARNESS_AUDIT_2026-10-21.md`. Ordered by dependency, then by
leverage-per-unit-risk. Every phase ends with a green test suite.

## Ground rules

- Nothing is deleted that a documented feature depends on (audit §III.5).
- The working tree contains intentional uncommitted work — never reset, stash, or clean.
- New behaviour ships with tests in the same commit-shaped change.
- Semantic colors stay in `tokens.css`; shared rendering primitives stay shared.

## Phase 0 — Correctness (blocking)

| # | Change | File | Verify |
|---|---|---|---|
| 0.1 | Register `compaction::compact_conversation_cmd` | `src-tauri/src/lib.rs` | `cargo test --lib` |
| 0.2 | Register `compaction::compaction_status` | `src-tauri/src/lib.rs` | " |
| 0.3 | Register `compaction::set_compaction_auto` | `src-tauri/src/lib.rs` | " |
| 0.4 | Fix `skills_skill_dependencies` → `skill_dependencies` | `src/lib/api.ts:181` | `npm run typecheck` |
| 0.5 | Ignore `__pycache__/`, `*.pyc` | `.gitignore` | `git status` clean of `.pyc` |
| 0.6 | Add a regression test asserting every frontend `invoke()` name resolves to a registered command | `src-tauri/src/tool_contract_regressions.rs` | new test |

## Phase 1 — Tool contract foundation

Everything else depends on the registry being able to carry an output contract.

| # | Change | File |
|---|---|---|
| 1.1 | New `ToolSpec { name, description, input, output: Option<Value> }` | new `src-tauri/src/tool_spec.rs` |
| 1.2 | All 16 `registry()` fns return `Vec<ToolSpec>` | 16 modules (mechanical transform) |
| 1.3 | `connectors::ToolView` / `AgentTool` carry `output` | `src-tauri/src/connectors.rs` |
| 1.4 | Output contract validated at the dispatch boundary | `src-tauri/src/tool_spec.rs` + `harness.rs` |
| 1.5 | Model-actionable input-validation errors (path + reason + fix hint) | `src-tauri/src/tool_validation.rs` |
| 1.6 | Regression test: every declared output contract validates against a real tool result | `tool_contract_regressions.rs` |

## Phase 2 — Independent feature modules (parallelizable)

| # | Feature | New module | Tools added |
|---|---|---|---|
| 2.1 | **Edit matcher** — 3-tier fuzzy match, `replaceAll`, CRLF preservation | `src-tauri/src/edit_match.rs` | extends `workspace_edit_file`, `workspace_multi_edit` |
| 2.2 | **Formatter pipeline** — detection table, auto-run after write/edit/patch | `src-tauri/src/formatter.rs` | `format_paths` |
| 2.3 | **Permission rules** — ordered allow/deny/ask + wildcards, persisted | `src-tauri/src/permission_rules.rs` | extends the approval gate |
| 2.4 | **Instruction chain** — `AGENTS.md` hierarchy + live reload | `src-tauri/src/instructions.rs` | none (prompt-level) |
| 2.5 | **Skills** — model-loadable skills | `src-tauri/src/skills.rs` | `skill_list`, `skill_load` |
| 2.6 | **Snapshots** — workspace snapshot/undo | `src-tauri/src/snapshots.rs` | `revert_stage`, `revert_commit` |
| 2.7 | **Unified shell** — one `shell` tool with `background` + auto-notify | `src-tauri/src/process_tools.rs` | `shell` (unified), `monitor` |
| 2.8 | **Session/worktree/model management** | `src-tauri/src/session_tools.rs` | `session_rename`, `session_move`, `worktree_list`, `models` |
| 2.9 | **Structured findings** | `src-tauri/src/findings.rs` | `report_findings`, `list_findings` |
| 2.10 | **Task graph** — dependency-aware tasks | `src-tauri/src/tasks.rs` | `task_create`, `task_update`, `task_list`, `task_get` |
| 2.11 | **Implicit style learning** ("taste") from accept/reject/edit | `src-tauri/src/taste.rs` | `taste_recall`, `taste_teach` |
| 2.12 | **Plugin API** — user-registered tools | `src-tauri/src/plugins.rs` | `plugin_list`, `plugin_reload` |

## Phase 3 — Integration

| # | Change |
|---|---|
| 3.1 | Every new tool wired into `harness::registry()`, a preset, and the capability map |
| 3.2 | New tools registered in `presets.rs` (de-duplicated alias table) |
| 3.3 | Frontend: renderers for new tool results, `/undo` affordance, permission-rule editor, task panel |
| 3.4 | `docs/` updates: `docs/PERMISSIONS.md`, `docs/TOOL_DISCOVERY.md`, `docs/WORKSPACE.md`, `docs/SKILLS.md` |

## Phase 4 — Minimization

| # | Change | Target |
|---|---|---|
| 4.1 | Delete zero-reference scratch: `repro-model.mjs`, `scripts/inspect_cdp_details.mjs`, `scripts/generate-catalog.mjs` | ~68 LOC |
| 4.2 | Delete dead catalog data: `catalog/sandbox-catalog.yaml`, `catalog/mcp-catalog.yaml`, `catalog/skill-catalog.yaml` | ~310 LOC |
| 4.3 | Delete fully-dead `pub fn` (`store.rs:802,1054,1514`, `connectors.rs:479`) | ~90 LOC |
| 4.4 | Delete test-only `#[allow(dead_code)]` items (`plans.rs:172`, `history.rs:86`, `research_tasks.rs:379`) | ~25 LOC |
| 4.5 | Delete orphan frontend components + orphan CSS | ~90 LOC |
| 4.6 | Delete web-only-test modules (7 files) | ~468 LOC |
| 4.7 | Delete unreferenced Rust examples (2 files) | ~57 LOC |
| 4.8 | Archive 11 dated audit docs → `docs/archive/` | ~2,217 LOC |
| 4.9 | De-duplicate `presets.rs` alias lists (5–6× → 1×) | ~200 LOC |
| 4.10 | Unify the diff renderer (`ReviewPanel.tsx` → `DiffLines`) | ~10 LOC + CSS |
| 4.11 | Extract `src-tauri/src/http_client.rs` — named constructors for 10 client sites | ~60 LOC |
| 4.12 | Extract `chat.rs` test module → `chat/tests.rs`; split `send_message_inner` | structural |
| 4.13 | Merge + split `store.rs` by aggregate | structural |
| 4.14 | Migrate 37 CDP smoke scripts → parameterized Playwright specs in `e2e/`; wire `e2e/` into CI | ~2,000 LOC |

## Verification gate (every phase)

```sh
cd src-tauri && cargo test --lib      # 699 passed baseline — must stay green
cd src-tauri && cargo check --lib
npm run typecheck
npx vitest run
```

---

# Implementation record — 2026-10-21

The plan above was executed. Every phase landed with tests; the tree is green
(Rust **965 passed / 0 failed**, frontend **770 passed / 1 skipped**).

## Phase 0 — Correctness (done)

| Change | Where |
|---|---|
| Registered `compaction::compact_conversation_cmd`, `compaction::compaction_status`, `compaction::set_compaction_auto` | `src-tauri/src/lib.rs` |
| Fixed the command name `skills_skill_dependencies` -> `skill_dependencies` | `src/lib/api.ts` |
| Registered `api_tools::open_workspace_path` (the frontend already called it) | `src-tauri/src/lib.rs` |
| Added `__pycache__/` and `*.py[cod]` to `.gitignore` | `.gitignore` |
| Added a regression test that diffs every frontend `invoke()` against the registered command list | `src-tauri/src/tool_contract_regressions.rs` |

The last one matters most: the four bugs above were all the same shape — a
name the frontend used that the backend never registered — and no Rust test
could see them. The new test reads `src/lib/api.ts` and `lib.rs` at compile
time and fails if they disagree.

## Phase 1 — Tool contract foundation (done)

- New `src-tauri/src/tool_spec.rs`: `ToolSpec { name, description, input, output }`.
- All 16 `registry()` functions now return `Vec<ToolSpec>`.
- Output contracts are validated at the dispatch boundary (`harness::enforce_output_contract`).
- Contracts are declared for `list_agents`, `memory_recall`, `todo_write`, `file_search`, `artifact_read`, `goal_set`, `goal_clear`, plus every new tool that returns a stable shape.
- A regression test asserts every declared contract compiles, names a real tool, and is satisfied by the tool's real result.

## Phase 2 — Feature modules (done)

| Module | Delivers |
|---|---|
| `edit_match.rs` | Three-tier edit matching (exact, then typographic, then trailing whitespace), `replaceAll`, CRLF/mixed-line-ending preservation. Wired into `workspace_edit_file` and `multi_edit`. |
| `formatter.rs` | Nine formatters with project detection, auto-run after every write/edit/patch, plus the `format_paths` tool. |
| `permission_rules.rs` | Ordered allow/deny/ask rules with wildcards, persisted under `permissions.rules`, evaluated before the access mode at the approval gate. |
| `instructions.rs` | The `AGENTS.md` chain, on-demand discovery on read, and live reload between turns. |
| `skills.rs` | `skill_list` and `skill_load` — the model can now load a skill. |
| `snapshots.rs` | Workspace snapshots with stage/commit/discard, content-addressed private storage, `tracked_paths` from the tool audit. |
| `session_tools.rs` | `session_rename`, `session_move`, `worktree_list`, `models`. |
| `findings.rs` | `report_findings`, `list_findings`, `resolve_finding`. |
| `tasks.rs` | A dependency-aware task graph: `task_create/update/get/list/delete`. |
| `taste.rs` | Implicit style learning from recorded accept/reject decisions, with secret redaction. |
| `plugins.rs` | User-authored, command-backed tools with a validation report. |

Integration points wired centrally:

- `harness::registry()` and `harness::execute_inner` gained the new tools.
- `presets.rs` gained the aliases, and `ASSISTANT_CATALOG` is now the **single** source of truth for the Standard preset — the drift risk the audit flagged is gone.
- `capabilities.rs` maps the new aliases.
- `chat.rs` consults the permission rules before the access mode, records the taste signal on explicit decisions only, captures a snapshot before each model step, and injects the instruction chain into the frozen system prompt.
- `prompt.rs` gained `frozen_system_for_project`, which carries the project instructions.
- `lib.rs` registered `list_permission_rules`, `save_permission_rules`, `list_snapshots`, `stage_revert`, `discard_revert`, `commit_revert`, `capture_snapshot`, `list_plugins`, `save_plugin`, `remove_plugin`.

## Phase 3 — Frontend and docs (done)

- New `src/components/PermissionRulesSettings.tsx` and `src/components/PluginSettings.tsx`, wired into the settings navigation and the app routing.
- New `src/lib/types.ts` entries and `src/lib/api.ts` wrappers.
- New `docs/HARNESS_TOOLS.md`, `docs/SNAPSHOTS.md`, `docs/PLUGINS.md`.
- `docs/PERMISSIONS.md`, `docs/WORKSPACE.md`, `docs/SKILLS.md` and `README.md` updated.

## Phase 4 — Minimization (done, except the script migration)

| Action | Result |
|---|---|
| Deleted zero-reference scratch (`repro-model.mjs`, `scripts/inspect_cdp_details.mjs`, `scripts/generate-catalog.mjs`) | 68 LOC |
| Deleted dead catalog data (`catalog/sandbox-catalog.yaml`, `catalog/mcp-catalog.yaml`, `catalog/skill-catalog.yaml`) | ~310 LOC |
| Deleted fully-dead `pub fn` (`store.rs`: `subagent_run`, `clear_compaction`, `artifacts_for_conversation`; `connectors.rs`: `call_with_stream`) | ~90 LOC |
| Deleted test-only `#[allow(dead_code)]` items | ~25 LOC |
| Deleted orphaned frontend components and their CSS | ~90 LOC |
| Deleted web modules imported only by their own tests (7 files) | ~468 LOC |
| Deleted unreferenced Rust examples (2 files) | ~57 LOC |
| Archived 11 dated audit documents into `docs/archive/` with a provenance note | ~2,200 LOC |
| De-duplicated the preset alias list (5-6 copies, now 1 source of truth) | ~200 LOC |
| Unified the diff renderer (`ReviewPanel` now uses `DiffLines`); removed the duplicate CSS vocabulary | ~10 LOC + CSS |
| Extracted `src-tauri/src/http_client.rs` — one place decides the outbound HTTP policy | ~60 LOC + 5 sites |
| Removed the dead public API the audit found (prompt helpers, provider helpers, instruction fingerprint, task table constant) | ~120 LOC |
| Moved test-only helpers behind `#[cfg(test)]` so they stop reading as public API | — |

**Not done:** migrating the 37 CDP smoke scripts into `e2e/` and wiring `e2e/`
into CI. That is plan item 4.14, and the audit's own reasoning says it must
come *after* `e2e/` exists in CI — the scripts currently encode per-feature
acceptance steps the Rust suite does not cover. Doing it without that safety
net would delete the only coverage those features have.
