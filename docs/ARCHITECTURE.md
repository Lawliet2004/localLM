# Architecture: DSH ideas adapted to LocalLM (feature/dsh-harness)

Agent = Model + Harness. Rust owns the security boundary; SQLite is durable;
the WebView never invokes shell directly.

## Capability seam (`capabilities.rs`)

Static catalog (`workspace_files`, `local_execution`, `cloud_execution`,
`system_time`, `skills_reader`). Source-backed entries resolve
per-conversation from `conversation_tools`; the rest from the global
`capabilities.disabled` setting. IPC: `list_capabilities`,
`set_capability_enabled` (operation-locked), `dump_config` (read-only, no
secrets). `chat.rs` skips `system_time` when disabled — temporal
composability without a config-file framework.

## Session log (`sessions.rs`, `SESSION_FORMAT_VERSION = 1`)

Model-visible ⟺ logged: `messages` + `runs`/`run_events` + `subagent_runs`
+ `todos`/`goals` + `compaction` checkpoints + `memory_facts`. New
model-visible input = new event (`context_injection`, `plan`,
`subagent_schedule`, `fork`, `compaction`, ...). Fork copies message rows
with fresh ids plus lineage; replay re-derives the transcript
(deterministic over rows; live re-runs are not). See
`docs/session-format-status.md`.

## Runtime modes (`presets.rs`)

Standard / Code (PTC) / Minimal / Creator are validated config patches over
tool visibility, stored per conversation (`conversation_presets`). The agent
loop, approval, and audit path are identical in every preset. Minimal =
execution + `edit_file` + `terminal_*` only. Code injects a generated TS SDK
(`ts_sdk`) and executes `ptc_run` step plans with per-step policy checks.

## Harness tools (`harness.rs` + `connectors.rs::ToolBackend::Harness`)

Subagents, todos, memory, terminal, web, schedules, PTC, Docker, plugins,
compaction are `AgentTool`s, so approval, audit rows, artifact bounding, and
`RunState` apply unchanged. Direct `call()` fails loud; only the dispatcher
runs them. The dispatcher returns pending run events the caller appends.

## Subagents (`subagents.rs`)

Bounded child loop (rounds × calls) with ephemeral history. Foreground
children reuse caller tools; background children run on supervisor tasks with
their own SQLite connection and local-only tools, settling via status rows +
parent tool-role notices. Depth cap errors loudly at the cap (tool stays
visible). `send_message` is exact-direct-parent only; `interrupt_agent`
rolls back pre-publication. Structured output is validated, not trusted.

## Plans (`plans.rs`)

`todo_write` (full replacement, ≤50), `goal_set`/`goal_clear`, injected as
logged plan state every turn. Workflows: sequential chains summaries,
parallel settles background children. Ralph: fixed objective, fresh child
per round, ` ```handoff ` fences, ≤5 rounds.

## Sandboxing (`sandbox.rs`)

Local execution is NOT an isolation boundary. Docker (`--network none`,
capped CPU/memory) is the opt-in boundary and fails loud without a daemon.
Persistent terminal sessions (create/send/resize/close, 64 KiB reads).
FS deny-list (`.ssh`, `.env`, private keys) hooks `workspace.rs::valid_path`;
binary/oversize guards. `web_fetch` (256 KiB + spill note), `file_search`
(fixed-string, 2000 files / 50 hits). Guards: loop-hygiene triple-call
breaker, configurable tool timeout.

## Memory (`memory.rs`)

SQLite bank, workspace scope or `global`. Verbatim recall blocks (4 KiB,
logged). `teach`/`forget` IPC + UI; `ingest_repo` does a read-only survey
(top-level layout + recent commits) as a self-healing auto fact.

## Scheduling (`scheduling.rs`)

Cron parser (`*`, `*/n`, ranges, lists), SQLite schedules, 30s runner,
unattended execution through the child loop with a reads-only pin (writes
need `allow_write` + conversation Full access). Loopback HTTP (127.0.0.1,
vault bearer, 64 KiB bodies): `POST /webhook/:id`, `GET /api/config`,
`GET /api/schedules`, `POST /api/schedules/run`. Headless CLI
(`--profile headless "task" --wait`) queues one-shot schedules over shared
SQLite/WAL. Python (`python/locallm_sdk.py`) and TS (`src/lib/sdk.ts`)
clients target the loopback API.

## Models (`inference.rs`, `providers.rs`)

`Backend::Local | OpenAi | Anthropic`. All backends normalize to
OpenAI-style SSE deltas (`StreamBody`), including Anthropic `tool_use` /
`input_json_delta` translation, so the loop is backend-agnostic. Formats:
`openai-chat-completions` (DeepSeek/GPT/Ollama/vLLM/Gemini-OAI) and
`anthropic-messages`. Context limits stay explicit; remote counting stays
estimated and labeled. Compaction (`compaction.rs`) checkpoints prefixes
into artifacts with cutoffs; auto mode is off by default and always logs.

## Plugins (`plugins.rs`)

Manifest shape only (name/version/capabilities/permissions/sandbox).
Plugins contribute configuration, never code execution. Install-from-folder
with hash pinning, enable/disable, regex malware scan
(install/caution/reject), in-memory creator testing. See `SAFETY.md`.

## Deliberately deferred

File-based preset overlays, `session_events` migration, ACP bridges,
`dsh-sdk` peer processes, DSBench/LM-Eval adapters, cloud memory sync.
Each has a named seam above; add on evidence of need.
