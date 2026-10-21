# Harness Audit — 2026-10-21

Reverse-engineering comparison of **localLM** against **OpenCode V2**, **Claude Code v2.1.x**, and **Command Code**.

Method: OpenCode's `packages/core/src/tool/**` was cloned (`anomalyco/opencode`, branch `dev`) and read at source level;
OpenCode V2 docs + OpenAPI were read via `opencode.ai/v2/**`; Claude Code and Command Code were read via
`code.claude.com/docs/**` and `commandcode.ai/docs/**`. localLM was read at source level (118 Rust files, 300 TS/TSX files).

**Nothing in the working tree was modified by this audit.**

---

## 0. Executive summary

| Dimension | localLM | OpenCode V2 | Claude Code | Command Code |
|---|---|---|---|---|
| Model-facing tools | **125** (100 harness aliases + 25 connector) | 26 | 45 | ~40 |
| Registry contract | `(name, description, input_schema)` | `(name, description, input, output, options, execute)` | internal | wire-level names |
| Output schemas | **none** | every tool | n/a | n/a |
| Plugin system | **none** | 16 transform domains + RPC + TUI slots | mods (`cmd.addTool`) | mods |
| Permission model | 3 coarse modes + capability kills | ordered rules + wildcards + policy tier | rule syntax + gitignore anchors | Claude-compatible modes |
| File-edit matching | exact only | 3-tier fuzzy + `replaceAll` + CRLF preserve | exact + `replace_all` | n/a |
| Formatter on write | **none** | 26 built-ins | hooks | n/a |
| Undo / rewind | **none** | snapshot (git object DB) | `/rewind` | checkpoints |
| Instruction files | filename filter only | `AGENTS.md` hierarchy + live reload | `CLAUDE.md` hierarchy | `AGENTS.md` |

**Headline verdict.** localLM is *tool-rich and mechanism-poor*. It has more model-facing tools than OpenCode and more
domain tooling (debugger, OpenAPI, database rehearsal, benchmarks, impact analysis, browser, computer-use) than OpenCode,
Claude Code, or Command Code combined. What it lacks is not *tools* — it is the **five load-bearing layers** that make
a tool surface safe and useful:

1. **No output schemas** → tool contracts are enforced by hand-written regression tests, not at runtime.
2. **No plugin surface** → the tool set is frozen at compile time; users cannot add a tool.
3. **No rule-based permissions** → 3 modes and a boolean "trusted read"; no path/command scoping, no wildcards.
4. **No undo/rewind** → a bad edit is only recoverable by the model re-deriving it.
5. **No formatter pipeline** → the agent writes code that is not formatted.

Separately, **4 Tauri commands the frontend calls are not registered** — the manual-compaction button, the
auto-compaction toggle, and the skill-dependency panel are dead at runtime.

---

# PART I — TOOL GAP ANALYSIS

## I.1 Verified live bugs (tools that exist but do not work)

Extracted the `tauri::generate_handler!` list (`src-tauri/src/lib.rs:285`, 190 commands) and diffed it against every
`invoke()` in `src/lib/api.ts` (169 distinct names).

**Frontend invokes a command the backend never registers — 4:**

| Frontend call | Site | Backend status |
|---|---|---|
| `compact_conversation_cmd` | `src/lib/api.ts:219` | `#[tauri::command]` at `src-tauri/src/compaction.rs:439` — **never registered** |
| `compaction_status` | `src/lib/api.ts:220` | `#[tauri::command]` at `src-tauri/src/compaction.rs:468` — **never registered** |
| `set_compaction_auto` | `src/lib/api.ts:221` | `#[tauri::command]` at `src-tauri/src/compaction.rs:481` — **never registered** |
| `skills_skill_dependencies` | `src/lib/api.ts:181` | registered as `skill_dependencies` (`src-tauri/src/lib.rs:331`) — **name mismatch** |

Impact: `src/components/Chat.tsx:378` (compact button), `src/components/ContextControl.tsx:44,89` (auto-compaction
toggle), and `src/components/Skills.tsx:41` (dependency list) all fail at runtime. These are *documented features*
(`docs/RENDERING.md`, `docs/CACHE_AND_TOOLS.md`, `docs/SKILLS.md`), so this is a regression, not dead code.

**Registered but never invoked by the frontend — 25:** `automation_diagnostics`, `cancel_subscription_sign_in`,
`delete_research_session`, `detect_subscription_cli`, `disconnect_subscription`, `dump_config`, `forget_fact`,
`get_conversation_runs`, `get_research_session`, `get_subscription_status`, `import_subscription_cli`, `ingest_repo`,
`interrupt_subagent`, `list_facts`, `list_research_sessions`, `list_subagent_models`, `list_subagent_runs`,
`sandbox_status`, `save_manual_subscription_token`, `set_sandbox_provider`, `skill_dependencies`,
`start_subscription_sign_in`, `teach_fact_cmd`, `web_search`, `web_search_health`.

These are reachable from Rust (model tool calls) but are **dead IPC surface** — 25 unauthenticated entry points into the
backend with no caller.

**Registered but unreachable by the model — 2:** `tool` (the echo/contract-probe tool at `harness.rs:78`) and `weather`
(the alias at `harness.rs:178`) are in the registry with live dispatch arms but appear in **no** preset's `harness`
list. `presets.rs:477` even asserts `!get(id).unwrap().harness.contains("tool")`. `tool_search`/`tool_invoke` are only
injected dynamically when the catalog exceeds 64 tools (`tool_discovery.rs:73-82`).

---

## I.2 Missing model-facing tools, ranked by leverage

### T1 — `edit` with fuzzy matching, `replaceAll`, and CRLF preservation

**OpenCode** (`packages/core/src/tool/plugin/edit.ts`, 332 lines) implements a **three-tier matcher**:

1. exact byte match;
2. else unicode-normalized match (smart quotes `''""` → `'"`, en/em dashes → `-`, NBSP → space);
3. else line-trimmed match (trailing whitespace ignored per line).

Plus:
- `replaceAll: boolean` — replace every occurrence, not just the first;
- **CRLF/mixed-line-ending preservation** via an LCS line-alignment (`alignLines`) that reuses each source region's own
  line endings; untouched bytes are never rewritten;
- `\r`-only differences apply to exact bytes (line-ending change mode);
- BOM preserved (`Bom.join`);
- `FileDiff.Info` returned as structured `output` + `metadata.files`.

**localLM** (`src-tauri/src/workspace.rs:704-790`, `edit_file`):
- exact `old_text` only — **zero** normalization, **zero** fuzzy fallback;
- **no `replaceAll`** — a multi-occurrence match is a hard error ("Use a longer unique match");
- UTF-8 only, no BOM handling, no CRLF preservation (rejects non-UTF-8 outright);
- 1 MiB file cap.

**Claude Code** `Edit` has `replace_all: true` and refuses non-UTF-8 explicitly (documented rationale: it would
replace undecodable bytes with `U+FFFD`).

**Gap:** the model's most common edit failure — the exact string it reconstructs differs by one smart quote, one
trailing space, or one CRLF — costs a full read+retry cycle. localLM turns a 1-token slip into a 3-tool round trip.

> **Recommendation:** port OpenCode's matcher verbatim (it is ~120 lines of pure functions, no I/O). Add
> `replaceAll` to the `workspace_edit_file` schema. Preserve existing CRLF.

### T2 — Formatter-on-write pipeline

**OpenCode** ships **26 built-in formatters** (`gofmt`, `mix`, `oxfmt`, `prettier`, `biome`, `zig`, `clang-format`,
`ktlint`, `ruff`, `air`, `uv`, `rubocop`, `standardrb`, `htmlbeautifier`, `dart`, `ocamlformat`, `terraform`,
`latexindent`, `gleam`, `shfmt`, `nixfmt`, `rustfmt`, `pint`, `ormolu`, `cljfmt`, `dfmt`), each gated on a project
detection heuristic (`prettier` requires a `prettier` dep in `package.json`; `biome` requires `biome.json` + installed
binary). The formatter runs **after every `write`, `edit`, and `patch`**. Custom formatters are config-driven
(`{command, extensions, environment, disabled}`), matched on the **final** extension, first success wins.

**localLM: zero.** `grep -rn "formatter\|rustfmt\|prettier" src-tauri/src/*.rs` returns only
`src-tauri/src/presentation.rs` (an unrelated LLM prompt formatter). Nothing formats code after an edit.

**Claude Code / Command Code** achieve the same via hooks (`PostToolUse` → shell command), not a built-in.

**Gap:** an agent that writes code and never formats it produces diffs that fail `cargo fmt --check` / `prettier --check`
in CI. This is the single highest-frequency quality defect in a coding harness.

> **Recommendation:** add `src-tauri/src/formatter.rs` with (a) a detection table mirroring OpenCode's, (b) a
> `format_file`/`format_paths` tool, (c) an automatic post-write hook on `workspace_write_file`, `workspace_edit_file`,
> `workspace_multi_edit`, `workspace_apply_patch`. Reuse `process_tools::registry`'s `shell_run` for the command.

### T3 — `skill` tool (model-loadable skills)

**OpenCode** (`packages/core/src/tool/plugin/skill.ts`): the model sees every permitted skill as `{id, name, description}`
and calls `skill` with an exact ID; OpenCode checks the agent's `skill` permission, strips frontmatter, injects the
body, and provides the base directory + up to 10 supporting file paths (contents are *not* auto-loaded).

**Claude Code** `Skill`: same model — "a skill runs through the existing `Skill` tool rather than adding a new tool entry."

**localLM:** has `skills_read_file` (`connectors.rs:357`) which reads a file *inside* a skill, and injects
`skills.instructions(&active_skills)` into the prompt at `chat.rs:1142`. But the **model cannot discover or load a
skill at runtime** — the active set is a user-side toggle only. The `skills_read_file` tool is a file reader wearing a
skill's name.

**Gap:** the model cannot say "use the PDF-extraction skill" or "use the release-notes skill". It must be pre-selected.

> **Recommendation:** add a `skill_load` harness tool (`{skill_id, path?}`) plus a `skill_list` tool. Fold
> `skills_read_file` into it (the `path` argument already implies it).

### T4 — `AGENTS.md` instruction hierarchy + live reload

**OpenCode V2** recognizes **`AGENTS.md` only** (no `CLAUDE.md` fallback). Scope: global `~/.config/opencode/AGENTS.md`
plus every `AGENTS.md` from cwd up to home/project root. Two mechanisms:

1. **On-demand discovery** — after a successful `read`, OpenCode walks *up* from the target to the Location root
   discovering `AGENTS.md` files and injects them as **durable synthetic instructions** (`read.ts`, `SessionInstructions.load`).
   Directory listing walks from the directory itself. External reads are skipped; discovery failures never fail the read.
2. **Live update** — edits to global/upward `AGENTS.md` are detected before the next model request and delivered as an
   "AGENTS.md changes" instruction update. Removing all ambient `AGENTS.md` tells the session they no longer apply.

Assembly order: agent/provider system prompt → Code Mode guidance → MCP/skill guidance → global+project `AGENTS.md` →
built-in date/environment → session instruction entries.

**localLM:** `AGENTS.md` appears in exactly one place — `src-tauri/src/source_inventory.rs:32` as a *filename filter*
in an inventory listing. There is no hierarchy load, no auto-discovery, no live reload. The repo's own `AGENTS.md`
(a 2,253-byte file with the verification commands) is therefore **never read by the agent** — it only reaches a human
developer.

**Gap:** this is the mechanism that makes an agent "know" the project's rules. Without it, `AGENTS.md` is decoration.

> **Recommendation:** (a) load the `AGENTS.md` chain into the system prompt at turn assembly
> (`chat.rs:1142`-adjacent, next to `skills.instructions`); (b) re-walk the chain each turn and diff it, so edits land
> mid-session; (c) on `workspace_read_file`, walk up from the read target and inject newly-found `AGENTS.md` files.

### T5 — Snapshot-based undo / rewind

**OpenCode** (`/v2/docs/snapshots/`): captures a snapshot **immediately before each model step** and another at step
completion. `/undo` (`<leader>u`) stages a rollback of the latest user message + work and returns the prompt to the
composer; submitting commits the rollback; `/redo` (`<leader>r`) cancels it. Files are restored from a **separate
internal Git object database** under the data dir — no commits, no branch moves, index untouched. Scope: tracked files,
non-ignored untracked ≤ 2 MiB each; git-ignored files and > 2 MiB files are excluded. Rollback is rejected while the
session is running. API: `POST /api/session/{id}/revert/stage`, `DELETE .../revert`, `POST .../revert/commit`.

**Claude Code**: `/rewind` checkpointing with code/file restores.

**localLM: nothing.** `grep -rn "undo\|rewind" src-tauri/src/*.rs` returns only unrelated matches (`agent_run.rs:104,126`
about pausing, `automation.rs` `browser_snapshot`). `store.rs:1075 fork_conversation` copies the *conversation*, not the
*filesystem*. `workspace_patch.rs` has a transaction journal with rollback — but that rollback is **internal to a single
multi-file transaction**, not user-addressable across turns.

**Gap:** when the agent corrupts a file, the only recovery is another agent turn (expensive, non-deterministic) or the
user's `git checkout` (destructive, loses the agent's other work). localLM's `workspace_patch` transaction machinery
means the *raw material* for snapshots already exists.

> **Recommendation:** snapshot the workspace at each model-step boundary into a content-addressed store (reuse
> `workspace_patch`'s sha256 + journal). Add `revert_stage` / `revert_commit` Tauri commands and a `/undo` action in the
> composer. This reuses existing primitives and is high-value.

### T6 — Session tree, boundary fork, session→worktree move

**OpenCode:** `Session.Info` carries `parentID` and `fork?: {sessionID, boundary}` where boundary is
`{type: "before" | "through", messageID}`. `session.fork` forks at a message boundary; `session.move` relocates a session
to a worktree/workspace; child sessions are navigable (`session.child.first/next/previous`, `session.parent`) and
exposed over ACP via `_meta["opencode/child-session"]`.

**localLM:** `store.rs:1075 fork_conversation` duplicates the whole conversation (no message boundary).
`worktree_enter`/`worktree_exit` exist (`git_tools.rs:158,161`) but **a session cannot move into a worktree**, and there
is no `worktree_list`.

**Gap:** "fork at message N and try a different approach" — the core experimental-ideation loop — is unavailable.

### T7 — Rule-based permissions (allow / deny / ask with wildcards)

**OpenCode** (`/v2/docs/permissions/`): ordered rules `{action, resource, effect}`, **last match wins**, wildcards `*`
and `?`, path normalization, per-agent rules, session-scoped rules layered after agent rules, and a
`Permission.evaluate` that hides a tool from the model entirely when denied by action.

**Claude Code**: `Bash(npm run *)`, `Edit(/src/**)`, `WebFetch(domain:example.com)`, `Skill(deploy *)`,
`Agent(Explore)`, `Read(~/secrets/**)`; gitignore path anchors (`//`, `~/`, `/`, relative); deny→ask→allow, first match
wins; bare tool name removes the tool from context; `!` negation carve-outs; redirects checked against `Edit`/`Read` rules.

**localLM** (`src-tauri/src/permissions.rs`, 121 lines): three `AccessMode` values (`Ask`, `AutoApprove`, `FullAccess`)
and a single boolean `trusted_read`. `automatic_reason(trusted_read)` auto-approves **trusted reads** only under
`AutoApprove`. There is **no rule syntax, no path scoping, no command prefix scoping, no wildcards, no "allow always
for this command"**.

Critically: `permissions.rs:11-59` (`PermissionClass`, `Grant`, `grants_for`, `backend_allows`) — the entire
**policy model** including the documented rule "Migration does not widen access. Ask stays ask… A model-written allow
never grants a permission" — is `#[cfg(test)]`-gated and **compiled out of the release binary**. The doc-comment
describes a policy that is not shipped.

**Gap:** a user cannot say "always allow `cargo test`" or "never allow writes outside `src/`". Every session re-prompts
for the same commands. This is the most-cited friction in every harness.

### T8 — Tool output schemas + runtime contract enforcement

**OpenCode** (`packages/core/src/tool/runtime.ts`, 281 lines): every tool declares `input` **and `output`** schemas
(Zod/Effect/Standard Schema). At the untrusted boundary, `runtime.execute`:
- decodes input against the declared schema → `Tool.Error` with a *model-actionable* message listing up to 5 issues
  with JSON paths plus the offending arguments;
- **enforces the declared output schema** — a tool that returns an invalid value fails with
  "Tool returned an invalid value for its output schema";
- tunnels foreign failures into `Tool.Error` so a call can never be left permanently unsettled.

**localLM:** the registry type is `Vec<(String, String, Value)>` — `(name, description, input_schema)`. **No tool
declares an output schema.** Contracts are enforced by hand-written regression tests
(`src-tauri/src/tool_contract_regressions.rs`, 349 lines; `ptc_contract_regressions.rs`, 493 lines) which are
*not* a runtime guarantee and do not cover the 25 connector tools.

**Gap:** a tool that returns a malformed result fails opaquely at some later point (or silently), rather than at the
boundary where it can be diagnosed.

> **Recommendation:** extend the registry tuple to `(name, description, input_schema, output_schema)` — a mechanical
> change across 18 `registry()` functions — and validate on return. This is the highest-leverage *mechanism* change in
> this report because it converts ~125 unwritten contracts into a checked invariant.

### T9 — `shell` as one tool with `background` + automatic completion notification

**OpenCode** `shell`: **one** tool — `{command, workdir, timeout, background}`. Default foreground timeout 2 minutes.
Background returns immediately with a shell ID and **the session is notified automatically when the command finishes**;
the tool description explicitly instructs the model *"You will be notified automatically… DO NOT poll for completion."*
A foreground timeout **moves the command to the background** rather than failing it. The command is parsed by a shell
scanner and permission is asserted **per scanner-produced command**, plus per inferred external directory.

**localLM** splits this into **8 tools**: `process_start`, `process_poll`, `process_write`, `process_stop`, `shell_run`,
`terminal_create`, `terminal_send`, `terminal_close`. `process_poll` exists precisely so the model can poll. There is
**no automatic completion notification** — a backgrounded process that finishes produces nothing until the model polls.

**Claude Code** `Monitor` goes further: it **streams each output line back into the conversation** so the model reacts
to log lines mid-turn, and it can also consume a **WebSocket** source. It is the only tool in any of the three harnesses
that gives the model a live event feed.

**Gap:** the model burns turns polling. OpenCode's explicit "DO NOT poll" is a direct answer to this and localLM has
no equivalent.

> **Recommendation:** add a `background` flag + `shell_id` to a unified `shell` tool, and a session-notification
> mechanism (localLM already has `research_progress`/`ProgressSink` plumbing in `web_search.rs:581`) that injects a
> synthetic turn when a background process settles.

### T10 — Plugin surface (add your own tools)

**OpenCode V2** `ctx.tool.transform(editor => { editor.namespace({name, description}); editor.add({name, description,
input, output, options, execute}) })` — plus transform/reload for **16 domains** (`agent`, `provider`, `model`,
`command`, `integration`, `credential`, `mcp`, `plugin`, `reference`, `skill`, `storage`, `tool`, `vcs`, `worktree`,
`websearch`, `session`, `permission`, `shell`, `event`, `rpc`, `generate`). Transforms are **replayable**; `reload()`
rebuilds without re-running `setup()`; registrations return an idempotent `dispose()`. Each model request captures a
**stable tool snapshot**. Plugin **RPC** adds typed custom methods + events over HTTP. CLI plugins add UI slots,
toasts, keymap layers, reactive stores.

**Claude Code / Command Code** both expose an in-process "mod" layer: `cmd.addTool`, `addCommand`, `addFlag`,
`addProvider`, `addRenderer`, `on`, `hooks`.

**localLM: none.** The tool set is compiled in. `presets.rs` selects *subsets* of a frozen list; `capabilities.rs` kills
*subsets* of a frozen list. There is no path for a user to add `deploy`, `jira`, `linear`, `figma`, or a
company-specific tool except by writing an MCP server.

**Gap:** this is the largest *extensibility* gap, and it is architectural, not incremental. Every domain tool
localLM ships (debugger, OpenAPI, database rehearsal, benchmarks) is a candidate for a user-provided plugin — localLM
paid for them in maintenance that users should be able to supply.

### T11 — Model-facing session/worktree management

**OpenCode** exposes `opencode_session_rename`, `opencode_session_move`, `opencode_worktree_create`,
`opencode_worktree_list`, `opencode_worktree_remove`, `opencode_models` — a namespaced tool family for managing the
harness itself.

**localLM:** `commands::rename_conversation` exists but is **Tauri-command-only, not model-facing**. `worktree_enter`/
`worktree_exit` exist but there is no `worktree_list`. `list_subagent_models` returns only an allowlist — no search, no
provider grouping, no variants, no cost, no paging.

**Gap:** the model cannot rename the conversation it is in, cannot move itself into a worktree, and cannot look up a
model ID to hand to a subagent (which is precisely what OpenCode's `models` tool exists for).

---

## I.3 Claude Code / Command Code tools localLM has no equivalent for

| Tool | What it does | localLM nearest | Verdict |
|---|---|---|---|
| `Monitor` | Streams background output lines back mid-conversation; also a WebSocket source | `process_poll` (manual polling) | **Missing.** High value — the only live event feed in any harness. |
| `ReportFindings` | Structured code-review findings (file, summary, failure scenario, category) | `change_impact` (semantic/lexical analysis) | **Missing.** The analysis exists; the *structured verdict tool* does not. |
| `TaskCreate/Get/Update/List` | Dependency-aware task **graph** | `todo_write`/`todo_add`/`todo_update` (flat list, no deps) | **Partial.** localLM's todos are flat with no dependency edges, no owner, no blocked-by. |
| `EnterPlanMode` / `ExitPlanMode` | Explicit mode switch into plan mode | `plan_submit` (approval request) | **Partial.** localLM has the approval, not the mode. |
| `Artifact` | Publish HTML/Markdown as an interactive page | `artifacts::bound_tool_result` (truncation spill only) | **Partial.** The artifact *store* exists; the *publish* surface does not. |
| `CronCreate/List/Delete` | Scheduled/recurring prompts | — | **Missing.** |
| `Workflow` | Dynamic multi-subagent workflow script | `workflow_run` (1–5 fixed stages) | **Partial.** localLM's is static; OpenCode/Claude's is code-defined. |
| `ScheduleWakeup` / `/loop` | Reschedule the next loop iteration | — | **Missing.** |
| `PowerShell` | Native PowerShell execution | `run_code` `language: powershell` | **Partial.** |
| `SendUserFile` | Send a file to the user's device | — | **Missing.** |
| `EndConversation` | Terminate the session | — | **Missing.** (Claude Code makes it unremovable on purpose.) |
| `PushNotification` | Desktop + phone push | — | **Missing.** (Desktop app has the surface.) |
| `ScheduleWakeup`, `WaitForMcpServers` | Wait for connecting MCP servers | `mcp_input` scopes | **Missing.** |

**Command Code's genuinely novel contribution — the `taste` tool.** Command Code ships `taste-1`, a *"meta
neuro-symbolic"* RL model that **continuously learns your coding style from every accept / reject / edit**, with taste
packages stored in `.commandcode/taste/` and shared via `npx taste push/pull`.

localLM has `memory_teach`/`memory_recall` — **explicit** facts the user states. It has nothing for **implicit**
learning: localLM *already captures* every accept/reject/edit decision in its approval flow
(`src-tauri/src/approval.rs`, `permissions.rs`) and throws that signal away. This is the most interesting idea in
Command Code that neither OpenCode nor Claude Code has, and localLM is unusually well-positioned to adopt because the
data is already being produced.

---

## I.4 The mechanisms localLM already has that OpenCode/Claude Code lack

These are assets. Any refactor must preserve them.

| localLM capability | Where | Why it is ahead |
|---|---|---|
| **PTC** — programmatic tool calling with a JSON AST | `ptc.rs` (2,105 lines), `harness.rs:1214` | OpenCode's `codemode` is a JS sandbox; localLM's is a bounded, auditable, budget-capped AST with 8 hard budgets (256 nodes / depth 8 / 128 statements / 64 iterations / **8 calls** / 60 s / 32 vars / 64 KiB). Safer than OpenCode's `execute`. |
| **`locallm.tool-envelope/1`** | `tool_envelope.rs` | Distinguishes `success / partial / failure / cancelled / unknown`. OpenCode has structured errors; Claude Code has exit codes. localLM's is a *versioned* contract with flat compat keys. |
| **Artifact-bounded results with live lifecycle handles** | `artifacts.rs` | `processId`, `proposalId`, `stagedTree` survive bounding. Neither reference harness has a truncation artifact model at all — OpenCode writes a file; Claude Code writes a file. |
| **Capability registry with dependency edges + transitive cascade** | `capabilities.rs`, `capabilities.json` | Fail-closed kill switch at 3 enforcement points (catalog assembly, dispatch, detached children). OpenCode has per-agent permissions; it has no *kill* concept. |
| **`expected_sha256` optimistic concurrency on every mutating tool** | `git_tools.rs`, `workspace_patch.rs`, `lsp_tools.rs` | Prevents TOCTOU corruption. Claude Code and OpenCode both permit blind writes. |
| **Transactional workspace writes with journal + rollback** | `workspace_patch.rs` (3,176 lines) | Multi-file atomic transactions with `recover_patch`. Nothing in OpenCode or Claude Code does multi-file atomic rollback. |
| **Debugger (DAP) tools** | `debug_tools.rs` | 5 DAP tools (`debugger_start/inspect/control/events/stop`). OpenCode dropped LSP entirely; Claude Code's LSP is a plugin with **no debugger**. |
| **OpenAPI + HTTP session + database rehearsal tools** | `openapi_tools.rs`, `api_tools.rs`, `database_tools.rs`, `database_rehearsal.rs` | localLM can call a REST API by OpenAPI spec and rehearse a migration on a disposable SQLite snapshot. Neither reference harness can. |
| **Benchmark / profile / verification / impact tools** | `performance_tools.rs`, `profile_tools.rs`, `verification_tools.rs`, `impact_tools.rs` | Speedscope + Chrome-trace profiling, before/after benchmark comparison, evidence-carrying check reports. Unique. |
| **Browser automation + computer use** | `automation.rs`, `computer_use.rs` | 11 browser tools + 7 computer-use tools, browser-side, not a remote service. |
| **Local-first / offline mode** | `local_only.rs` | Both reference harnesses are cloud-first. |

---

# PART II — JUNK FILE AUDIT

## II.1 Disk (all git-ignored, none load-bearing)

| Path | Size | Git state | Verdict |
|---|---|---|---|
| `src-tauri/target/` | **13 G** | ignored (`src-tauri/.gitignore:3`) | legit build output |
| `.local/` | **53 M** | ignored (`.gitignore:21` — `*.local`) | runtime captures; `.local/qa/` alone is 50 M |
| `graphify-out/` | **15 M** | ignored (`.gitignore:38`) | extracted repo knowledge graph; **no build or CI reference** |
| `.impeccable/` | **11 M** | `review/` ignored (`:41`); `critique/` **tracked** | 192 review captures + 1 tracked one-off critique |
| `dist/` | 3.2 M | ignored (`:19`) | legit (`tauri.conf.json → frontendDist`) |

## II.2 Junk that is *tracked* or *nearly* tracked

| Item | Evidence | Action |
|---|---|---|
| **`scripts/__pycache__/` (4 `.pyc`)** | `git status` shows `?? scripts/__pycache__/benchmark-generative-ui.cpython-314.pyc` etc. | **`.gitignore` has NO `__pycache__` / `*.pyc` rule** (`grep -c "pycache\|pyc" .gitignore` → 0). One `git add -A` from being committed. |
| **`src-tauri/tests/fixtures/__pycache__/lsp_fixture.cpython-314.pyc`** | `??` in `git status` | same |
| **`src-tauri/resources/web/worker.mjs` — 258 KB generated blob, tracked** | generated by `scripts/build-web-worker.mjs` → `src-tauri/resources/web`; consumed by `include_str!` at `src-tauri/src/web_search.rs:7` | Must exist at compile time, but a 258 KB generated file in the index is a footgun. Generate in `build.rs`, or mark it clearly generated. |
| **`src-tauri/src/genui_prompt.txt` — 98 lines, tracked, generated** | `scripts/build-genui-prompt.mjs:24` writes it; `src-tauri/src/prompt.rs:29` `include_str!`s it | same |
| **`.impeccable/critique/2026-10-17T07-23-15Z__src-components-chat-tsx.md`** | tracked (`A`); one-off tool output, `total_score: 24`, 0 references | archive or delete |
| **`.windsurf/rules/graphify.md`** | tracked; instructs agents to read `graphify-out/graph.json` — **which is gitignored and absent from a fresh clone** | vestigial |
| **`catalog/sandbox-catalog.yaml`** | **0 references anywhere in the repo** | delete |
| **`catalog/mcp-catalog.yaml`, `catalog/skill-catalog.yaml`** | referenced only by `scripts/generate-catalog.mjs` (itself unreferenced) | delete with the script |
| **`catalog/skills.lock.json` (27.8 KB)** | `include_str!` at `src-tauri/src/skills.rs:74` | **KEEP — live** |
| **`catalog/runtime-assets.json`** | `include_str!` at `src-tauri/src/runtime_archive.rs:24` | **KEEP — live** |
| **`catalog/TRUEFORGE-LICENSE`, `catalog/PROVENANCE.md`** | third-party licence obligations (`docs/RELEASE.md:54`) | **KEEP** |

## II.3 One-off scratch scripts with zero references

| File | LOC | References | Verdict |
|---|---|---|---|
| `repro-model.mjs` (repo root) | 19 | **0** — reads a nonexistent `/tmp/audit-api-model.js`, hardcodes a user-specific GGUF path | **junk** (and tracked) |
| `scripts/inspect_cdp_details.mjs` | 43 | **0** — hardcodes conversation UUID `fa01ad24-…` | **junk** |
| `scripts/generate-catalog.mjs` | 6 | **0** — writes `src/lib/catalog.json`, but the live file is `include_str!`'d and tracked | **junk (stale)** |

## II.4 The `scripts/` sprawl

**85 files, 6,489 lines. 44 are `*-smoke.mjs` (~2,600 lines).**

- Only **1** script runs in CI: `scripts/development-acceptance.py` (`.github/workflows/development-tools.yml:60`).
  There is exactly **one** workflow file in the repo.
- Only **12** scripts are reachable from `package.json`.
- **43 scripts have zero package.json / CI references.**
- **37 scripts** independently call `connectOverCDP('http://127.0.0.1:9223')`; **35** write `test-results/*.json`;
  each re-implements the same teardown (`save_preferences` restore, `delete_conversation`,
  `localStorage.removeItem('locallm-draft:'+id)`, `unload_model`) — see
  `scripts/context-indicator-smoke.mjs:33-39` vs `scripts/permissions-smoke.mjs` vs `scripts/local-mcp-chat-smoke.mjs`.
- `e2e/` — **21 Playwright specs, 1,700 lines** — **never runs in CI**
  (`grep -rn "playwright\|e2e" .github/workflows/` → 0 hits). `playwright.config.ts` exists and is tracked.
- `scripts/installer-smoke.ps1`, `prepare-runtime.ps1`, `window-smoke.ps1` (162 lines) — Windows-only, no Windows CI.

**The duplication problem is not the scripts — it is that they duplicate the Rust test suite.** `scripts/workspace-edit-smoke.mjs`
(124 lines) re-tests `workspace_patch.rs` transactions (`workspace_patch.rs:1791-3175`). `scripts/skill-dependencies-smoke.mjs`
re-tests `src-tauri/src/skills.rs`. The Rust suite already has **699 passing tests**.

**Recommendation:** migrate the 37 CDP scripts into parameterized Playwright specs under `e2e/`, wire `e2e/` into CI,
then delete the scripts. Net: **~2,000 LOC removed**, but a real safety net added.

## II.5 `docs/` — 51 files, 5,374 lines

- **19** referenced from `README.md`; **3** more referenced only from code.
- **30** referenced from neither README nor code, and **11** of those are not cross-linked from any other doc.
- **2,354 LOC (44%)** is dated audit/implementation-plan churn. Notably `INFERENCE_TOOL_AUDIT_2026-10-19.md` (249) and
  `INFERENCE_TOOL_AUDIT_CURRENT_2026-10-19.md` (95) are the same audit in two versions.
- Keep `IMPLEMENTATION_REPORT.md` and `REVIEW_REMEDIATION_2026-10-17.md` — README-linked.

## II.6 Orphaned frontend files (import-graph verified)

**Zero production importers:**

| File | LOC | Note |
|---|---|---|
| `src/components/ComposerOptions.tsx` | 42 | its CSS classes don't exist anywhere → fully dead |
| `src/components/VoiceInput.tsx` | 43 | `.voice-input` CSS orphaned at `src/desktop.css:469-471` |
| `src/web/cache/embedding_cache.ts` | 27 | |
| `src/web/models/llm_provider.ts` | 26 | |
| `src/web/security/url_policy.ts` | 19 | |
| `src/web/verticals/vertical_provider.ts` | 7 | |

**Imported only by `*.test.*` (dead in the shipped bundle):**

| File | LOC | Note |
|---|---|---|
| `src/web/tools.ts` | 163 | `WebAgentTools` class — not exported by `src/web/index.ts`, not imported by `scripts/web-worker.mjs` |
| `src/web/verification/structured_result.ts` | 123 | |
| `src/web/evaluation/evaluator.ts` | 103 | |

## II.7 Duplicated React components

**Two diff vocabularies.** `src/components/DiffView.tsx` exports the shared `DiffLines` (used by `CommandRunCard.tsx`,
`WorkSummary.tsx`, `src/genui/Coding.tsx`). But `src/components/ReviewPanel.tsx:91` re-implements the diff inline with
*different class names* (`diff-add`/`diff-remove`/`diff-hunk` vs `diff-line add`/`del`/`hdr`) → **two CSS sets for one
concept**. Fix: `<DiffLines diff={git.diff} />`.

`CodeBlock.tsx` is already unified (fan-in 6) — no action.

---

# PART III — DEAD RUST CODE & MINIMIZATION

## III.1 Dead / false-public Rust symbols

**Fully dead `pub fn` (zero references anywhere in the crate, including tests):**

| Location | Symbol |
|---|---|
| `src-tauri/src/connectors.rs:479` | `pub async fn call_with_stream` |
| `src-tauri/src/store.rs:802` | `pub fn subagent_run` |
| `src-tauri/src/store.rs:1054` | `pub fn clear_compaction` |
| `src-tauri/src/store.rs:1514` | `pub fn artifacts_for_conversation` |
| `src-tauri/src/compaction.rs:439` | `pub async fn compact_conversation_cmd` — **do not delete; register it** |
| `src-tauri/src/compaction.rs:468` | `pub async fn compaction_status` — **do not delete; register it** |
| `src-tauri/src/compaction.rs:481` | `pub async fn set_compaction_auto` — **do not delete; register it** |

**83 further `pub fn` are referenced only inside their own file** — `pub` is a false claim of public API. Largest
clusters: `src-tauri/src/subscription_auth.rs` (10), `src-tauri/src/store.rs` (6), `src-tauri/src/providers.rs` (6),
`src-tauri/src/subagents.rs` (5), `src-tauri/src/skills.rs` (2), `src-tauri/src/capabilities.rs` (3),
`src-tauri/src/tool_envelope.rs` (4).

Of 1,017 `pub fn` scanned: **90 (8.9%)** are dead or falsely public.

## III.2 `#[allow(dead_code)]` — 13 total, 3 hide real dead code

- `src-tauri/src/plans.rs:172` `make_todo` — used **only** by tests.
- `src-tauri/src/history.rs:86` `model_history` — used **only** by tests (production uses `model_history_with_cutoff`).
- `src-tauri/src/research_tasks.rs:379` `_storage_contract` — **never called at all**.
- The 8 in `src-tauri/src/tool_envelope.rs` are the *builder* half of the envelope API
  (`Status::as_str/parse`, `make`, `success`, `error_envelope`, `validate_envelope`, `legacy_execution_to_envelope`) —
  production only uses `normalize` / `normalize_error` / `failure_notice` / `failed`.

## III.3 Duplicated implementations

### (a) 14 independent HTTP client policies, no shared module

| Site | Policy |
|---|---|
| `src-tauri/src/download.rs:68` | `https_only(true)`, redirect limit 10, connect 15 s, **no total timeout** |
| `src-tauri/src/api_tools.rs:156` | `retry::never()`, custom redirect policy, timeout 30 s — **no `https_only`** |
| `src-tauri/src/api_tools.rs:186` | as above + cookie provider |
| `src-tauri/src/providers.rs:559` | redirect `none()`, connect 10 s, timeout **180 s** |
| `src-tauri/src/connectors.rs:911` | connect 10 s, timeout 60 s, redirect `none()` |
| `src-tauri/src/daytona.rs:79` | default headers, redirect `none()` |
| `src-tauri/src/inference.rs:278` | `no_proxy()`, redirect `none()`, connect timeout |
| `src-tauri/src/skills.rs:493` | timeout 45 s, redirect `none()` |
| `src-tauri/src/openapi_tools.rs:133` | redirect `none()`, timeout 15 s |
| `src-tauri/src/sandbox.rs:631` | redirect `none()`, connect timeout |
| `src-tauri/src/mcp_http.rs:325` | `BoundedHttpClient` — a *separate* bounded wrapper |

**Two HTTP paths coexist**: `mcp_http::BoundedHttpClient` (wire/header limits) and 10 raw `reqwest::Client::builder()`
sites each with its own redirect/timeout policy. `api_tools.rs` lacks `https_only` while `download.rs` has it —
an inconsistency in a security-relevant knob.

> **Recommendation:** `src-tauri/src/http_client.rs` exposing named constructors (`model_provider()`, `mcp()`,
> `download()`, `tool_api()`) with **named knobs** — do not unify the values (180 s for model providers is correct; no
> total timeout for large downloads is correct).

### (b) Two workspace-edit engines

| Engine | Location | Semantics |
|---|---|---|
| **Assistant (transactional)** | `src-tauri/src/workspace.rs:567` → `workspace_patch.rs` (3,176 lines) | `cap_std::Dir`, journal at `.locallm-transactions`, `root_lock`, preflight, rollback, manifest, `expected_sha256` pinning, `multi_edit`/`edit_lines`/`write_file`/`apply_patch` |
| **Editor (direct)** | `src-tauri/src/editor.rs` (604 lines) | `cap_std` but **no journal, no rollback, no transaction lock**; own sha256 pinning, own `MAX_BYTES = 2 MiB` |

`editor.rs:1-3` states the intent ("deliberately separate from the assistant's file tools"), so this may be
intentional — but both independently implement path validation, sha256 pinning, atomic rename, and root bounding.
Worth confirming the intent is still valid.

### (c) Four tool-registry mechanisms

1. 18 per-module `registry()` functions (leaf definitions)
2. `connectors.rs:692 tools_for_turn` (turn catalog assembly)
3. `chat.rs:1685 freeze_tool_catalog` (freeze + persist)
4. **`presets.rs:171 assistant_catalog() -> [&'static str; 64]`** — a hard-coded static list of 64 aliases that
   **duplicates** the registry output

**The same ~56-alias block appears 5–6× in `presets.rs`** (verified: `lsp_execute_command` ×6, `git_commit` ×6,
`lsp_rename` ×6, `verification_run` ×5). `presets.rs` contains **515 quoted alias tokens** for 7 presets. This is a
live drift risk: adding a tool to a preset list does not add it to `assistant_catalog()`.

> **Recommendation:** one `const ASSISTANT_ALIASES: &[&str]` + per-preset `const`s built from it with `+`/`-` slices.
> Removes ~200 LOC and the drift.

### (d) Four parallel approval mechanisms

| Mechanism | Location | Scope |
|---|---|---|
| `Approvals` (bool allow/deny) | `approval.rs:5-47` + `chat.rs:3619` + `harness.rs:1289` + `subagents.rs:1489` | user allow/deny |
| `ChoiceApprovals` (option choice) | `approval.rs:50-88` + `harness.rs:853,880` | `ask_user` |
| `AccessMode` policy | `permissions.rs:5-10`, `automatic_reason` | auto-approve decision |
| Capability kills | `capabilities.rs:375` + `chat.rs:1880` | gating |

Plus hooks (`hooks.rs`, 473 lines). Four mechanisms, one decision.

## III.4 Structural bloat

### `src-tauri/src/chat.rs` — 6,725 lines

- 136 fn definitions, 23 `match` blocks, 116 match arms.
- **`send_message_inner` at `chat.rs:2537-4240` = 1,704 lines (25% of the file)** — 170 `if`s, 264 `let`s, 13 `await`s.
  It contains the model loop, tool dispatch, approval gate, plan-mode gate, verification-before-finish, loop hygiene,
  and final-answer assembly.
- `assemble_turn` at `chat.rs:1003-1684` = 682 lines (catalog assembly + prompt freezing + message building).
- `chat.rs:4602-6724` = 2,124 lines of inline tests.

**Extraction seams, safest first:**
1. `chat.rs:4602-6724` → `src-tauri/src/chat/tests.rs` (or `src-tauri/tests/chat.rs`) — mechanical, zero risk.
2. `chat.rs:2537-4240` → `chat/loop.rs` + `chat/approval_gate.rs` + `chat/tool_dispatch.rs` + `chat/finalize.rs`.
3. `chat.rs:1003-1684` → `chat/assemble.rs`.
4. `chat.rs:45-228` (`ThinkFilter`, `ResearchProgress`) → `chat/filters.rs`.

### `src-tauri/src/store.rs` — 3,620 lines

- 145 fn definitions, **83 SQL call sites**.
- Two `impl Store` blocks: `store.rs:208-1859` (1,652 lines) and `store.rs:1951-2536` (586 lines), split only by a free
  function `default_subscription_providers` at `store.rs:1860-1950` — an **arbitrary** cut, not a conceptual one.
- `store.rs:2541-3619` = 1,080 test lines.

**Seam:** merge the two impl blocks, then split by aggregate into
`store/{conversations,messages,providers,tools,research,sessions}.rs`.

### Files whose trailing `#[cfg(test)] mod` exceeds 40% — extraction candidates

| File | production | test | % test |
|---|---|---|---|
| `src-tauri/src/tool_discovery.rs` | 244 | 537 | **68%** |
| `src-tauri/src/agent_run.rs` | 267 | 489 | **64%** |
| `src-tauri/src/mcp_protocol.rs` | 432 | 689 | **61%** |
| `src-tauri/src/download.rs` | 440 | 572 | 56% |
| `src-tauri/src/api_tools.rs` | 422 | 476 | 53% |
| `src-tauri/src/provider_capabilities.rs` | 542 | 551 | 50% |
| `src-tauri/src/inference.rs` | 793 | 807 | 50% |
| `src-tauri/src/process_tools.rs` | 760 | 777 | 50% |
| `src-tauri/src/lsp_tools.rs` | 1,015 | 832 | 45% |
| `src-tauri/src/providers.rs` | 915 | 731 | 44% |
| `src-tauri/src/workspace_patch.rs` | 1,790 | 1,386 | 43% |
| `src-tauri/src/check_tools.rs` | 926 | 688 | 42% |

`src-tauri/src/permissions.rs` is the extreme case: **100 of 121 lines are `#[cfg(test)]`-gated** — the shipped binary
contains only `AccessMode` and `automatic_reason`.

`src-tauri/tests/` contains **zero `.rs` files** — only 3 Python fixtures. All Rust tests are inline.

## III.5 Minimization plan, ordered by impact/risk

### Tier 0 — correctness (0 LOC saved, unblocks everything)

| # | Action | Location | Risk |
|---|---|---|---|
| 0.1 | **Register the 3 compaction commands** | `src-tauri/src/lib.rs` — add `compaction::compact_conversation_cmd`, `compaction::compaction_status`, `compaction::set_compaction_auto` | **Very low.** Fixes a live bug |
| 0.2 | **Fix the skill-dependency command name** | `src/lib/api.ts:181` → `'skill_dependencies'` | **Very low.** One string |
| 0.3 | **Add `__pycache__/` + `*.pyc` to `.gitignore`** | `.gitignore` | None |

### Tier 1 — zero-risk deletions (~1,200 LOC)

| # | Action | Paths | LOC |
|---|---|---|---|
| 1.1 | Delete tracked one-off scratch | `repro-model.mjs`, `scripts/inspect_cdp_details.mjs`, `scripts/generate-catalog.mjs` | 68 |
| 1.2 | Delete dead catalog data | `catalog/sandbox-catalog.yaml`, `catalog/mcp-catalog.yaml`, `catalog/skill-catalog.yaml` | ~310 |
| 1.3 | Delete fully-dead `pub fn` | `store.rs:802,1054,1514`; `connectors.rs:479` | ~90 |
| 1.4 | Delete test-only `#[allow(dead_code)]` items | `plans.rs:172`, `history.rs:86`, `research_tasks.rs:379` | ~25 |
| 1.5 | Delete orphan frontend components + orphan CSS | `ComposerOptions.tsx`, `VoiceInput.tsx`, `desktop.css:469-471` | ~90 |
| 1.6 | Delete web-only-test modules | `src/web/tools.ts`, `verification/structured_result.ts`, `evaluation/evaluator.ts`, `cache/embedding_cache.ts`, `models/llm_provider.ts`, `security/url_policy.ts`, `verticals/vertical_provider.ts` | 468 |
| 1.7 | Delete unreferenced Rust examples | `src-tauri/examples/credential_store_smoke.rs`, `model_metadata_smoke.rs` | 57 |
| 1.8 | Delete tracked one-off critique | `.impeccable/critique/2026-10-17T07-23-15Z__src-components-chat-tsx.md` | — |

### Tier 2 — high-impact, low-risk consolidations (~2,400 LOC)

| # | Action | Detail | LOC | Risk |
|---|---|---|---|---|
| 2.1 | **Archive dated audit docs** → `docs/archive/` | 11 files (see II.5) | **2,217** | Low — keep the 2 README-linked ones |
| 2.2 | **De-duplicate `presets.rs` alias lists** | `presets.rs:63,87,129,171-237` → one `ASSISTANT_ALIASES` const + per-preset slices | ~200 | Low — pure refactor; removes real drift risk |
| 2.3 | **Unify the diff renderer** | `ReviewPanel.tsx:91` → `<DiffLines/>`; drop `diff-add/remove/hunk` CSS | ~10 + CSS | Low |
| 2.4 | **Extract a shared HTTP-policy module** | new `http_client.rs`; replace 10 builder sites with *named* constructors | ~60 | **Medium** — unify the knobs, not the values |
| 2.5 | **Extract `chat.rs`'s test module + seams** | `chat.rs:4602-6724` → `chat/tests.rs`; `send_message_inner` → `chat/loop.rs` | 0 LOC, makes a 4,601-line file reviewable | Medium — mechanical move |
| 2.6 | **Merge + split `store.rs`** | one `impl Store`, then `store/{conversations,messages,providers,tools,research,sessions}.rs` | 0 LOC | Medium |

### Tier 3 — script consolidation (~2,500 LOC, do last)

| # | Action | LOC | Risk |
|---|---|---|---|
| 3.1 | Migrate the 37 CDP smoke scripts → parameterized Playwright specs in `e2e/`; wire `e2e/` into CI; then delete the scripts | ~2,000 | **High** — they encode per-feature acceptance steps the Rust suite doesn't cover. Migrate first, delete second. |
| 3.2 | Delete Windows-only PS scripts if no Windows CI is added | 162 | Medium |
| 3.3 | Delete orphaned ATPI/seed Python helpers | ~460 | Medium — some referenced from `docs/DEVELOPMENT_COMPLETION.md` |

### Tier 4 — archive / ignore

`graphify-out/`, `.local/`, `.impeccable/review/`, `dist/` are already ignored ✅. Add
`src-tauri/resources/web/worker.mjs` + `src-tauri/src/genui_prompt.txt` to a "generated files" convention (or move
generation into `build.rs`).

### Expected outcome

| Tier | LOC saved | % of repo text | Risk |
|---|---|---|---|
| Tier 1 | ~1,200 | 0.8% | None / Low |
| Tier 2 | ~2,400 | 1.6% | Low–Medium |
| Tier 3 | ~2,500 | 1.7% | **High** — last |
| **Total** | **~6,100** | **~4.1%** | |

**Structural (no LOC change):** `chat.rs` 6,725 → ~2,900; `store.rs` 3,620 → 5 files; `presets.rs` alias table 5–6× → 1×;
`permissions.rs` 121 → ~60 with the model actually shipped.

**Disk reclaimed (all already git-ignored):** 13 G (`target/`) + 78 M (`graphify-out` + `.local` + `.impeccable`).

### ⚠️ Where minimization would break a documented feature

| Item | Documented claim | Correct action |
|---|---|---|
| `compaction::compact_conversation_cmd` / `compaction_status` / `set_compaction_auto` | `docs/RENDERING.md`, `docs/CACHE_AND_TOOLS.md`; UI at `ContextControl.tsx:44,89`, `Chat.tsx:378` | **Register, do not delete.** |
| `skills::skill_dependencies` | `docs/SKILLS.md`; UI at `Skills.tsx:41` | **Fix the frontend string.** |
| `permissions.rs` `PermissionClass`/`Grant`/`grants_for`/`backend_allows` | `docs/PERMISSIONS.md` + README:60 | ⚠️ The **documented 3-mode behavior** lives in `AccessMode` + `automatic_reason` — that must stay. The test-only grant model may go, **but confirm with a maintainer**: it is possible the test model is the *intended* policy and the production path is the incomplete one. |
| `catalog/skills.lock.json`, `catalog/runtime-assets.json` | `docs/RELEASE.md`, `docs/SKILLS.md` | **KEEP** — `include_str!`. |
| `catalog/TRUEFORGE-LICENSE`, `catalog/PROVENANCE.md` | licence obligations | **KEEP.** |
| `docs/IMPLEMENTATION_REPORT.md`, `docs/REVIEW_REMEDIATION_2026-10-17.md` | README-linked | **KEEP** in place. |
| `src-tauri/resources/web/worker.mjs`, `src-tauri/src/genui_prompt.txt` | `include_str!` | **KEEP.** |
| `e2e/` (1,700 LOC) | `playwright.config.ts` exists | **KEEP** — but wire into CI before touching the smoke scripts. |
| `src-tauri/src/subagents.rs`, `subscription_auth.rs`, `memory.rs` | exposed as model tools via `presets.rs` | **KEEP the modules**; only the 25 unused Tauri *command wrappers* may go. |
| `scripts/development-acceptance.py` | the only CI job | **KEEP.** |

---

# PART IV — RECOMMENDED ROADMAP

Ordered by leverage-per-unit-risk.

| # | Action | Why | Est. effort | Leverage |
|---|---|---|---|---|
| **1** | Register the 3 compaction commands; fix `skills_skill_dependencies` | Fixes 4 dead UI features | 15 min | **Critical** |
| **2** | Port OpenCode's 3-tier edit matcher + add `replaceAll` + CRLF preservation | Removes the #1 agent failure mode | 2–3 h | **Very high** |
| **3** | Add a formatter pipeline (26 built-ins + config) with auto-run after write/edit/patch | Every coding agent needs this | 4–6 h | **Very high** |
| **4** | Add `output_schema` to the registry + validate at the boundary | Converts 125 unwritten contracts into an invariant | 6–10 h | **Very high** |
| **5** | Load the `AGENTS.md` chain into the system prompt; re-walk each turn | Makes the repo's own AGENTS.md reach the agent | 2–3 h | **High** |
| **6** | Add a `skill_load` / `skill_list` tool | Lets the model select skills at runtime | 2 h | **High** |
| **7** | Rule-based permissions (`{action, resource, effect}` + wildcards) replacing the 3 modes | Removes the re-prompt treadmill | 1–2 days | **High** |
| **8** | Workspace snapshot/undo (reuse `workspace_patch` sha256 + journal) | Recovery without agent re-derivation | 2–3 days | **High** |
| **9** | Unified `shell` tool with `background` + automatic completion notification | Stops the model polling | 1–2 days | **Medium-high** |
| **10** | `AGENTS.md`-style on-demand instruction discovery on `read` | Nested-monorepo correctness | 1 day | **Medium** |
| **11** | Session boundary fork + `session_move` + `worktree_list` + model-facing `session_rename` | Experimentation loop | 2 days | **Medium** |
| **12** | A plugin/tool-registration API (`ctx.tool.transform` equivalent) | Lets users contribute the domain tools localLM maintains | 3–5 days | **Medium-high (architectural)** |
| **13** | `Monitor`-style live event feed for background processes | Only harness with mid-turn reactivity | 2 days | **Medium** |
| **14** | `ReportFindings`-style structured review tool | localLM has the analysis, not the verdict channel | 1 day | **Medium** |
| **15** | Dependency-aware task graph (`TaskCreate/Update` superset of `todo_*`) | Replaces the flat todo list | 2 days | **Medium** |
| **16** | **Implicit style learning** from accept/reject/edit (Command Code's `taste` idea) | localLM already produces the signal and discards it | 3–5 days | **Medium-high (novel)** |
| **17** | Minimize: Tier 0 + Tier 1 + Tier 2 | ~3,600 LOC, low risk | 1–2 days | — |
| **18** | Minimize: Tier 3 (scripts → e2e → CI) | ~2,500 LOC, higher risk | 2–3 days | — |

---

# APPENDIX — Verification commands

```sh
# tool registry + dispatch
grep -rn "pub fn registry" src-tauri/src/            # 18 registry functions
grep -n "pub async fn execute" src-tauri/src/harness.rs # primary dispatcher (harness.rs:400)

# the 4 broken Tauri commands
grep -n "compact_conversation_cmd\|compaction_status\|set_compaction_auto" src/lib/api.ts src-tauri/src/lib.rs
grep -n "skill_dependencies" src/lib/api.ts src-tauri/src/lib.rs

# dead code
grep -rn "allow(dead_code)\|allow(unused)" src-tauri/src/
grep -c '"' src-tauri/src/presets.rs                   # 515 quoted alias tokens

# junk
git status --porcelain --untracked-files=all | grep -i "pycache\|\.pyc"
grep -rn "playwright\|e2e" .github/workflows/         # 0 hits — e2e never runs
grep -rn "catalog/sandbox-catalog\|sandbox-catalog" --include='*.rs' --include='*.ts' .  # 0 hits

# sizes
du -sh src-tauri/target graphify-out .local .impeccable dist
```

**OpenCode source references used:**
`packages/core/src/tool/runtime.ts` (contract enforcement),
`packages/core/src/tool/plugin/{read,edit,write,patch,shell,grep,glob,question,subagent,todo,skill,webfetch,websearch,mcp-resource,opencode}.ts`,
`packages/core/src/tool/{read-filesystem,html-markdown,http-body,mcp}.ts`,
`packages/core/src/tool/patch.txt`, `packages/core/src/tool/todowrite.txt`.
Docs: `opencode.ai/v2/docs/{tools,permissions,policies,agents,skills,commands,instructions,references,formatters,snapshots,api,mcp-servers,session}/`,
`opencode.ai/v2/openapi.json` (143 operations).
Claude Code: `code.claude.com/docs/en/{tools-reference,permissions,permission-modes,sandboxing,hooks,sub-agents,agent-teams,skills,mcp,checkpointing,plugins/mods}`.
Command Code: `commandcode.ai/docs`, `github.com/CommandCodeAI/command-code`.

---

## Follow-up — implemented 2026-10-21

The gaps this audit identified were closed. See
[`IMPLEMENTATION_PLAN_HARNESS.md`](IMPLEMENTATION_PLAN_HARNESS.md) for the
plan and the implementation record, and [`HARNESS_TOOLS.md`](HARNESS_TOOLS.md)
for what the model can now reach.

| Audit finding | Resolution |
|---|---|
| Four frontend `invoke()` names with no registered command | Fixed, and a test now diffs the two lists so the class cannot recur. |
| Registry cannot express an output contract | `ToolSpec` with an output schema, enforced at the dispatch boundary. |
| Edit matching is exact-only | `edit_match.rs`: three-tier matching, `replaceAll`, CRLF preservation. |
| No formatter after write/edit/patch | `formatter.rs`: nine formatters with project detection, plus `format_paths`. |
| No rule-based permissions | `permission_rules.rs`: ordered allow/deny/ask rules, evaluated before the access mode. |
| `AGENTS.md` is only a filename filter | `instructions.rs`: chain load, on-demand discovery on read, live reload. |
| No model-loadable skills | `skill_list` and `skill_load`. |
| No undo | `snapshots.rs`: stage/commit/discard over content-addressed private storage. |
| No model-facing session/worktree/model tools | `session_rename`, `session_move`, `worktree_list`, `models`. |
| No structured review verdict | `report_findings` / `list_findings` / `resolve_finding`. |
| Flat todo list | `tasks.rs`: a dependency-aware task graph. |
| No implicit style learning | `taste.rs`: learns from recorded accept/reject decisions. |
| No plugin surface | `plugins.rs`: user-authored, command-backed tools. |
| Preset alias list duplicated 5-6 times | One `ASSISTANT_CATALOG` const; the Standard preset derives from it. |
| 14 ad-hoc HTTP client policies | `http_client.rs`: one place decides the outbound policy. |
| ~90 dead or falsely-public Rust symbols | Removed, or moved behind `#[cfg(test)]`. |
| Junk files and one-off scratch | Deleted; 11 dated audits archived with a provenance note. |
