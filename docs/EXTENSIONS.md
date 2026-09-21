# Extension Catalog

Candidate features that fit the existing LocalLM harness. This is a design
menu, not a commitment: nothing here is scheduled. Entries marked
**Status** have been started; everything else is unimplemented. Outstanding work on the *current* scope lives in
[SPEC.md](SPEC.md), [PROGRESS.md](PROGRESS.md) and
[HARNESS-IMPROVEMENT.md](HARNESS-IMPROVEMENT.md) — finish those before
starting anything below.

Each entry records **What / Why here / Where it plugs in / Effort / Watch
out**. Effort is rough: **S** ≈ under a day, **M** ≈ a few days, **L** ≈ one
to three weeks, on top of the existing architecture.

## Contents

- [What already exists](#what-already-exists)
- [Invariants every extension must respect](#invariants-every-extension-must-respect)
- [Tier 1 — Inference quality on a small local model](#tier-1--inference-quality-on-a-small-local-model)
- [Tier 2 — Agent loop reliability](#tier-2--agent-loop-reliability)
- [Tier 3 — New tools and surfaces](#tier-3--new-tools-and-surfaces)
- [Tier 4 — Operations, UX and evidence](#tier-4--operations-ux-and-evidence)
- [Suggested sequencing](#suggested-sequencing)
- [Deliberately not recommended](#deliberately-not-recommended)

## What already exists

Read this first; several obvious ideas are already shipped and are not
repeated as proposals.

**Runtime.** Managed `llama-server` child process on authenticated loopback
(`runtime.rs`), with `--ctx-size`, `--n-gpu-layers`, `--threads`,
`--batch-size`, `--ubatch-size`, `--flash-attn`, `--cache-type-k/v`,
`--no-kv-offload`, `--parallel`, `--jinja`, `--mmproj`
(`runtime_config.rs`). Vision projectors are already supported. Pinned
runtime/model download, SHA-256 verification, cancellation and install
recovery (`download.rs`, `model_install.rs`, `install_recovery.rs`,
`runtime_install.rs`).

**Prompt.** Prefix-stable layout with `cache_prompt: true`, frozen tool
schemas per conversation (`conversation_prompt_freeze`), `id_slot` pinning
for parent vs. subagent (`prompt.rs`, `inference.rs`).

**Harness tools** (`harness.rs::registry`): `web_search`, `web_open`,
`web_find`, `web_fetch_url`, `web_fetch`, `file_search`, `todo_write`,
`todo_add`, `todo_update`, `goal_set`, `goal_clear`, `subagent`,
`send_message`, `interrupt_agent`, `list_agents`, `list_subagent_models`,
`workflow_run`, `ralph_run`, `terminal_create/send/resize/close`,
`memory_teach`, `memory_recall`, `ask_user`, `artifact_read`, `ptc_run`,
`docker_exec`, `preset_guide`, `compact_conversation`,
`research_pause/resume/cancel/progress`, plus the AREX control set
(`search`, `visit`, `update_context`, `finish`) in `arex.rs`.

**Local sources.** Workspace file tools (`list_files`, `read_file`,
`create_file`, `edit_file`) under cap-std, local `run_code` /
`execute_command`, Daytona cloud execution, remote MCP connectors (14
presets) and local stdio MCP servers.

**Orchestration.** Capability microkernel with presets
(`capabilities.rs`, `capabilities.json`, `presets.rs`: standard, code,
minimal, creator, chat, research, coding), subagents with depth limits,
personas, `output_schema` and background runs (`subagents.rs`), append-only
`session_events` with forking and replay (`store.rs`, `sessions.rs`),
compaction and checkpoints (`compaction.rs`), long-running research tasks
(`research_tasks.rs`), versioned structured tool envelopes
(`tool_envelope.rs`), artifacts for truncated output (`artifacts.rs`),
three-mode approval (`approval.rs`, `permissions.rs`).

**Providers.** OpenAI-compatible, ChatGPT/Grok subscription, FreeToken
loopback, and native Claude Messages (`providers.rs`,
`claude_adapter.rs`, `subscription_auth.rs`).

**Research stack** (`src/web/`): query planning, SearXNG retrieval with
pagination and engine diagnostics, fetch transport with SSRF/redirect/robots
enforcement, extraction, chunking, dedup/freshness/source-quality ranking,
reranking, evidence state, verification and evaluation, verticals
(time/weather/currency), observability.

**Persistence.** SQLite tables: `conversations`, `messages`,
`session_events`, `runs`, `run_events`, `subagent_runs`, `todos`, `goals`,
`memory_facts`, `artifacts`, `compaction`, `research_sessions`,
`research_tasks`, `conversation_tools`, `conversation_presets`,
`conversation_prompt_freeze`, `providers`, `settings`.

## Invariants every extension must respect

Any feature below has to hold the lines the harness already draws, or it is
a regression however useful it looks:

1. **Model-visible ⟺ logged.** Anything sent to or emitted by the model is
   durably committed to `session_events` (ARCHITECTURE.md §2). A new tool
   that injects context without an event breaks replay and forking.
2. **Prefix stability.** New prompt content goes in the trailing volatile
   section, never between the frozen system message and frozen tool
   schemas, or every `cache_prompt` hit is lost (`prompt.rs`).
3. **One approval path.** Presets are config patches over tool visibility,
   not alternative executors (`presets.rs` header). A new capability must
   route through the same policy check and audit.
4. **No silent remote calls.** Local-first; remote providers only through
   explicitly configured, verified connections.
5. **No secrets in prompts or logs.** Credentials stay in `vault.rs` behind
   the Windows Credential Manager key.
6. **Local execution is not an isolation boundary** and must never be
   presented as one (SPEC.md).
7. **No fabricated success.** No synthetic metrics, no fixture result
   presented as live verification.

---

## Tier 1 — Inference quality on a small local model

The default target is a 2B–7B quantized GGUF. The largest available wins are
in decoding and context handling, not in more tools.

### 1.1 Grammar-constrained decoding for tool calls and structured output

**What.** Convert each tool's JSON Schema into a GBNF grammar (or pass
`json_schema` directly) on the `llama-server` request, so the model is
*incapable* of emitting a malformed tool call or an off-schema final
answer.

**Why here.** The single highest-leverage change for small models. The
harness already owns every schema (`harness.rs::registry`,
`tool_discovery.rs`), already validates envelopes after the fact
(`tool_envelope.rs`), and already carries a `output_schema` field on
`subagent`. Today malformed calls are caught by `tool_calls.rs` parsing and
retried; constrained decoding removes the failure class instead of
recovering from it.

**Where.** `inference.rs` (the payload at `chat.rs:1957` currently sends
only `temperature`, `top_p`, `max_tokens`, `stream`, `cache_prompt`); a new
`grammar.rs` for schema→GBNF; `tool_envelope.rs` for the validated result
contract.

**Effort.** M for tool calls, S more for final-answer schemas.

**Watch out.** Grammar changes with the active tool set, so it must be
frozen alongside `conversation_prompt_freeze` or it churns the cache.
Remote providers (`providers.rs`) have no equivalent — gate per backend and
fall back to current parse-and-retry, reporting which path ran.

**Status (2026-09-21): final-answer schemas done; tool calls stay with the
runtime.** The runtime is launched with `--jinja` (llama.cpp b10855), and for
supported chat templates llama-server already compiles the request's `tools`
schemas into a lazy grammar. A harness-side schema→GBNF converter for tool
calls would therefore duplicate or conflict with it, so none was added. The
harness never sends `grammar`/`response_format` together with `tools`, and the
grammar follows the frozen tool set automatically. `structured_output.rs` covers what the
runtime cannot know: a subagent's `output_schema`. If the last tool-free
answer fails the schema, one repair round runs with no tools.
- On the local runtime it carries `response_format: json_schema`, which
  llama-server compiles into a grammar.
- If the runtime rejects the schema, it falls back once to an unconstrained
  retry and records why.
- Remote providers get the unconstrained retry.

Each repair is logged as a `structured_output` run event with
`decoding: grammar|unconstrained`, the model-visible prompt and the outcome.
The schema is now checked against the last round's answer, not text
accumulated across rounds. Not yet done:
- Templates without a native tool handler still fail with the text-form
  tool-call error rather than falling back to a generic constrained format.
- llama-server does not report per request whether a tool grammar was
  applied, so that remains unobservable.
- AREX `finish` answers are validated but not repaired.
- The before/after measurement with §2.5 is still to run.

### 1.2 KV slot save/restore for instant session resume

**What.** Use `llama-server`'s slot save/restore (`--slot-save-path`,
`/slots/{id}?action=save|restore`) to persist a conversation's KV cache to
disk and reload it, so reopening a long chat does not reprocess the whole
prefix.

**Why here.** The prefix-stable prompt work (ARCHITECTURE.md §1b) already
guarantees a byte-identical prefix; this is the payoff. On a 4 GB-VRAM
target, prompt reprocessing dominates time-to-first-token for long chats.

**Where.** `runtime_config.rs` (new flag), `inference.rs` (slot lifecycle),
`store.rs` (slot file path per conversation), `sessions.rs` (invalidate on
fork/compaction).

**Effort.** M.

**Watch out.** Slot files are invalidated by any model, runtime or
tool-schema change — key the cache on model hash + freeze hash and delete
aggressively on mismatch. Disk growth needs a budget and a purge path, and
the files hold conversation content, so they belong under the same privacy
statement as `locallm.sqlite` (README "Data and privacy").

**Status (2026-09-21): implemented, not yet measured.**
- **Enabling:** `runtime.rs` passes `--slot-save-path <data>/kv-slots` only
  when the runtime's `--help` lists it.
- **Save and restore:** `kv_slots.rs` saves slot 0 after each completed turn.
  Before a turn, it restores the conversation's file if another conversation
  has used the slot since. Both are logged as ignorable `kv_slot` session
  events with the measured tokens, bytes and milliseconds.
- **Cache key:** files are keyed on model, projector and runtime
  path+size+mtime plus the full launch config. Hashing the weights on every
  load was rejected as too slow. A mismatch deletes the file.
- **Invalidation:** tool-schema and compaction changes are deliberately not
  keyed. llama.cpp reuses only the common token prefix after a restore, so a
  stale file costs reuse, never correctness, while the frozen system and tool
  prefix still benefits.
- **Storage and privacy:**
  - An LRU purge enforces a disk budget (default 4 GB).
  - Files are deleted with their conversation.
  - Turning the feature off deletes every file.
  - A settings panel shows the file count and size and has a purge button.
  - The README privacy statement is updated.

Known limit: with one inference slot, a background subagent can occupy slot 0
between turns. The next turn then skips the restore and reprocesses, which is
slower but correct. Still to do: measuring the resume speed-up on the target
hardware with §2.5 (`kvRestore` and `cacheHitRatio` are now scored).

### 1.3 Draft-model speculative decoding

**What.** Load a small draft model (`--model-draft`, `--gpu-layers-draft`)
alongside the main model to accelerate generation.

**Why here.** `model_library.rs` already manages multiple local GGUFs and
`runtime_config.rs` already surfaces per-field runtime tuning, so this is
mostly configuration plumbing plus a second verified download.

**Where.** `runtime_config.rs`, `runtime.rs`, `model_library.rs`,
`RuntimeForm.tsx`, `catalog/runtime-assets.json` for the pinned draft
weights.

**Effort.** M.

**Watch out.** Draft and target must share a tokenizer/vocabulary; validate
at load and fail loud rather than silently ignoring the flag. Measure before
claiming a speedup — on a 4 GB card the extra VRAM may cost more than the
speculation saves, and `docs/PROGRESS.md` requires recorded evidence.

### 1.4 Local embeddings plus a vector index

**What.** A second `llama-server` instance (or the same one with
`--embedding`) serving an embedding GGUF, backed by a `sqlite-vec` /
brute-force cosine index over workspace files, memory facts and archived
research documents.

**Why here.** Three existing subsystems are currently limited to literal
matching or recency: `file_search` is fixed-string only (`sandbox.rs:580`,
2000 files / 50 hits), `memory_recall` is scope+limit over `memory_facts`,
and compaction archives verbatim history that nothing can later query
semantically. This also unlocks §2.2 and §3.4.

**Where.** New `embeddings.rs` + `vector_store.rs`; new SQLite table;
consumers in `sandbox.rs` (`file_search`), `memory.rs` (`memory_recall`),
`compaction.rs`, and `src/web/reranking/`.

**Effort.** L.

**Watch out.** Don't bolt semantic search onto the existing tool names
silently — add `semantic_search` / `memory_search` so the model (and the
audit log) can tell which retrieval ran. Indexing a workspace must respect
the same cap-std root and `.gitignore`, and must be incremental.

### 1.5 Full sampler surface and reproducible seeds

**What.** Expose `top_k`, `min_p`, `repeat_penalty`, `presence/frequency
penalty`, `mirostat`, `dry_*`, `logit_bias` and an explicit `seed`, saved as
named generation profiles selectable per conversation and per preset.

**Why here.** Only `temperature`, `top_p` and `max_tokens` reach the model
today (`chat.rs:1957`). A fixed `seed` is also a prerequisite for the eval
harness (§2.5) and for deterministic replay (§2.7) to mean anything.

**Where.** `chat.rs`, `inference.rs`, preferences in `store.rs`,
`presets.rs`, `Models.tsx` / `RuntimeForm.tsx`.

**Effort.** S for the parameters, M for profiles and per-preset defaults.

**Watch out.** Sampler fields are not portable across providers; the Claude
and OpenAI adapters must drop unsupported fields explicitly rather than
passing them through and hoping.

**Status (2026-09-21): parameters done, profiles pending.** `store::Sampling`
(`topK`, `minP`, `repeatPenalty`, `presencePenalty`, `frequencyPenalty`,
`seed`) is nested under `Preferences.sampling`, optional per field, validated,
and applied in `chat.rs::request_payload` and both subagent loops.
`inference.rs` drops `top_k`/`min_p`/`repeat_penalty` for OpenAI-compatible
providers and all extended fields for subscription providers; the Claude
adapter forwards only `top_k`. Exposed under Models → Generation → Advanced
sampling. Not yet done: `mirostat`, `dry_*`, `logit_bias`, named profiles and
per-preset defaults.

### 1.6 Per-preset LoRA adapters

**What.** Hot-swap LoRA adapters (`--lora`, `/lora-adapters`) so a preset
can carry a specialised adapter — e.g. a tool-calling-tuned adapter for
Coding, a citation-tuned adapter for Research.

**Why here.** Presets are already the unit of behavioural configuration;
adapters are the weight-level equivalent, and `llama-server` can scale them
at runtime without a reload.

**Where.** `runtime.rs`, `runtime_config.rs`, `presets.rs`,
`model_library.rs` for pinned adapter provenance.

**Effort.** M.

**Watch out.** Adapter provenance and hashes belong in
`catalog/runtime-assets.json` with the same verification as model weights.
Swapping an adapter invalidates KV slots (§1.2) and prompt caches.

### 1.7 Prompt-cache and throughput telemetry

**What.** Surface `timings` from `llama-server` (prompt tokens processed vs.
cached, tokens/s prefill and decode, slot id) as a per-turn panel and a
rolling chart.

**Why here.** The prompt-freeze machinery is invisible today; a cache-hit
readout turns "prefix stability" from an architectural claim into an
observable number, and it is the measurement instrument for §1.1–§1.3.

**Where.** `inference.rs` (capture), `store.rs` (per-message timing row),
`ActivityTimeline.tsx` / `HardwareStatus.tsx`.

**Effort.** S–M.

**Watch out.** Report measured values only; if a provider does not return
timings, show "unavailable", never an estimate.

**Status (2026-09-21): per-turn readout done, chart pending.**
`telemetry.rs` parses the `timings` object from the final streamed chunk:
`cache_n`, `prompt_n`, prefill and decode times and rates, and generated
tokens. Only reported values are kept. Each round emits an ignorable
`timings` session event plus a live `ChatEvent.timings`. The composer shows
"Last round: N prompt tokens (M from cache, X%) · prefill … · decode …", or
"unavailable for this provider" for remote backends. The eval harness now
scores output tokens and cache-hit ratio from these events. Not yet done: the
rolling chart, and a `HardwareStatus` view of slot state.

---

## Tier 2 — Agent loop reliability

### 2.1 Lifecycle hooks (pre-tool / post-tool / on-turn-end)

**What.** User-configurable hooks that fire around tool calls and turn
boundaries: run a formatter after `edit_file`, run tests after a code
change, block a tool by policy, inject context before a turn.

**Why here.** `capabilities.rs` already models lifecycle `effects` on the
Registry — the seam exists and is unused. Hooks are how users encode
workflow rules without patching Rust.

**Where.** `capabilities.rs` (effect dispatch), `chat.rs` agent loop,
`approval.rs` (a hook may deny), new `hooks.rs` + a settings-backed
config, `session_events` for hook execution records.

**Effort.** M–L.

**Watch out.** A hook is arbitrary local execution: it needs the same
approval semantics and audit as `execute_command`, a timeout, bounded
output, and it must not be able to escalate a denied tool into an allowed
one. Hook failures must fail the step loudly, not silently continue.

### 2.2 Workspace checkpoint and rollback

**What.** Snapshot the workspace before a run (or before each write tool),
show a cumulative diff, and offer one-click revert of everything an agent
changed.

**Why here.** The agent can already write and execute; the only recovery
today is the per-edit diff mentioned in the README, and SPEC.md still lists
"file and artifact previews" as outstanding. A shadow git repository (or a
content-addressed copy under app data) gives a real undo without touching
the user's own VCS state.

**Where.** New `checkpoints.rs`; `workspace.rs` write paths;
`artifacts.rs` for diff storage; `WorkspacePanel.tsx` / `WorkSummary.tsx`.

**Effort.** M.

**Watch out.** Never commit to, or rewrite, the user's real git repository.
Snapshot size needs a cap and a retention policy; binary and large files
should be excluded by rule and the exclusion reported.

**Status (2026-09-21): implemented; git behaviour checked with git 2.49, Rust
not yet compiled.**
- **Snapshots:** `checkpoints.rs` snapshots the workspace into a shadow repository
  (`<data>/checkpoints/<hash>.git`, selected with `--git-dir`) before and
  after every turn that offers a tool able to modify it. That includes
  shell and code execution, which a per-tool journal would miss.
- **Isolation:** user and system git config, hooks and fsmonitor are disabled
  for the shadow repository, and pathspecs are literal.
- **Exclusions:** `.gitignore` is respected. Files over 5 MB and nested
  repositories are excluded and reported. More than 20,000 new files or
  1 GB skips the checkpoint with a stated reason and never blocks the turn.
- **Revert:** revert is per turn and path by path. A path is restored only if
  its current content is still what the turn left; otherwise it is reported
  as a conflict. The revert is itself recorded as a checkpoint.
- **Retention and privacy:** the latest 100 turns per workspace are kept, with
  refs pruned and `gc` run afterwards. There is a UI toggle and a
  delete-all, and the README privacy statement covers the copies.
- **UI:** Review → Changes by turn.

Known limits:
- Empty directories left behind by a revert are not removed.
- Deleting a conversation removes its rows, but its shadow objects stay
  until the next pruning pass in that workspace.

### 2.3 First-class git tools

**What.** `git_status`, `git_diff`, `git_log`, `git_branch`, `git_commit`
as audited harness tools rather than shell strings through `terminal_send`.

**Why here.** Coding is a first-class preset; today every git operation goes
through a free-form shell that always requires approval and yields
unstructured text. Typed tools give structured output for the model,
granular permissions (read-only git under Auto-approve reads, `git_commit`
always asking), and a clean audit record.

**Where.** `harness.rs::registry` + a `git.rs` executor,
`harness.rs::is_trusted_read` for the read-only subset, `presets.rs`
(Coding, Standard).

**Effort.** M.

**Watch out.** Commits, pushes, branch deletion and history rewriting must
stay outside the trusted-read set and outside Full-access auto-approval
unless the user explicitly opts in per repository.

**Status (2026-09-21): implemented, without the per-repository opt-in.**
- **Read tools:** `git.rs` provides `git_status` (porcelain v2 parsed),
  `git_diff` (staged, base or path; 64 KiB cap), `git_log` and `git_branch` as
  trusted reads in the Standard, Coding and Creator presets.
- **`git_commit`:** always asks. `AgentTool::always_asks` overrides Full access
  in the chat loop and in `ptc_run`, and subagents are refused because they
  have no approval surface.
- **Deliberately absent:** push, reset, checkout, rebase, amend and branch
  deletion.
- **Argument checks:** revisions and paths are validated so they cannot become
  options or leave the workspace. Inherited `GIT_*` variables are cleared.

The per-repository opt-in for unattended commits is not built; commits
always ask.

### 2.4 Tool-result memoisation

**What.** Cache tool results keyed by `(tool, canonical args, workspace
revision)` within a run, returning a cached envelope with an explicit
`cached: true` flag instead of re-executing.

**Why here.** `sandbox.rs::check_repetition` already detects repeated
calls and blocks loops; caching is the constructive version of the same
signal. Small models re-read the same file repeatedly, and every repeat
costs a full prompt round-trip.

**Where.** `sandbox.rs`, `tool_envelope.rs`, `store.rs` (run-scoped cache
table), `chat.rs` loop.

**Effort.** M.

**Watch out.** Only cache verifiably pure reads. Never cache execution,
never cache across an intervening write to the same path, and always mark
cached results in the envelope and the trajectory so the audit stays
honest.

### 2.5 Evaluation and regression harness

**What.** A task suite (fixture workspace + prompt + assertion) run across
presets and models, producing a scored report: task success, tool-call
validity rate, citation correctness, tokens, wall time, approval count.

**Why here.** The Minimal preset exists explicitly "for benchmarks" but
there is no benchmark. `scripts/*-smoke.mjs` covers mechanism, not agent
quality. Every Tier 1 proposal above is a claim about quality that
currently cannot be substantiated, and PROGRESS.md demands before/after
evidence on the same task.

**Where.** New `scripts/eval/` runner plus a `evals/` fixture directory;
reuses `session_events` for scoring and `src/web/evaluation/` for the
research metrics.

**Effort.** L.

**Watch out.** Requires §1.5 (seed) for reproducibility. Report variance
across repeats, not a single run; a small model's run-to-run spread will
exceed most of the deltas being measured.

**Status (2026-09-21): runner, scorer and starter suite done; no baseline yet.**
`scripts/eval/run.mjs` drives the debug app over CDP, one fresh fixture copy
and conversation per (preset, task, repeat), with a fixed seed and an
explicit per-task approval policy. `score.mjs` derives pass/fail, tool-call
validity, tool errors, denials, approvals, peak input tokens and wall time
from `session_events`. `report.mjs` reports mean ± sample SD, and `merge.mjs`
compares models. There are five starter tasks in `evals/tasks/`, and the
format is documented in `evals/README.md`. `npm run test:eval` covers the
scorer offline. Not yet done: citation-correctness scoring via
`src/web/evaluation/`, output-token accounting (the harness does not record
it; see §1.7/§4.1), and a recorded baseline run on the target hardware.

### 2.6 Run budget governor

**What.** Per-run ceilings on tokens, wall time, tool calls and subagent
depth, with a declared degradation path (compact → summarise → stop and
report) instead of an abrupt failure.

**Why here.** The loop is already bounded by round count and depth
(`subagents.rs`, `HarnessCtx::depth_limit`) but not by token or time spend,
and `ralph_run` / `workflow_run` / background subagents can multiply that
cost invisibly. Remote providers make it a money question as well.

**Where.** `agent_run.rs`, `chat.rs`, `subagents.rs`, `compaction.rs`
(degradation), surfaced in `ActivityTimeline.tsx`.

**Effort.** M.

**Watch out.** Hitting a budget must produce a partial result with an
explicit reason recorded in `session_events`, in the same spirit as the
existing response-token-limit handling.

### 2.7 Deterministic replay against a different model or preset

**What.** Re-run a recorded trajectory from `session_events` with tool
results stubbed from the log (or re-executed), under a different model,
preset or sampler profile, and diff the two trajectories.

**Why here.** This is nearly free given the append-only log, forking and
lineage already implemented (ARCHITECTURE.md §3) — it needs a replay driver
and a diff view, not new persistence. It is the natural debugging tool for
"why did it do that" and the natural consumer of §2.5.

**Where.** `sessions.rs` (replay driver), `Trajectory.tsx` (side-by-side
diff), `agent_run.rs`.

**Effort.** M.

**Watch out.** Replayed runs must be clearly marked as replays in the log
and must never re-execute a side-effecting tool without a fresh approval.

### 2.8 Plan-mode approval

**What.** A mode where the model first proposes an ordered plan of tool
calls, the user approves or edits the whole plan once, and the loop then
executes it with deviations requiring re-approval.

**Why here.** `plans.rs` already parses plan steps into todos and the
PlanChecklist UI already renders them; `ptc_run` already executes a
policy-checked multi-step program. This joins the two into a permission
mode that sits between "Ask for approval" (prompt fatigue) and "Full
access" (no oversight).

**Where.** `permissions.rs` / `approval.rs` (fourth mode), `plans.rs`,
`ptc_run` in `harness.rs`, `PlanChecklist.tsx`.

**Effort.** M.

**Watch out.** The approved plan is the authorisation scope; any argument
change at execution time must invalidate it and re-ask, or this becomes
Full access wearing a plan.

### 2.9 Shared blackboard for multi-agent runs

**What.** A conversation-scoped, append-only key/value scratchpad that
parent and subagents can read and write (`note_write`, `note_read`), so
`workflow_run` parallel branches can share findings without funnelling
everything through summary handoffs.

**Why here.** Subagents today communicate only via prompt, bounded
handoffs and `send_message` to a direct parent (`harness.rs`,
`subagents.rs`). A blackboard makes parallel fan-out genuinely
collaborative, and it is a small table plus two tools.

**Where.** `store.rs` (new table), `harness.rs::registry`,
`subagents.rs`.

**Effort.** S–M.

**Watch out.** Scope strictly to the conversation, size-cap entries, and log
every write as a `session_event` — otherwise it becomes an unlogged side
channel into the model's context and breaks the §1 invariant.

---

## Tier 3 — New tools and surfaces

### 3.1 MCP server mode (expose LocalLM's tools outward)

**What.** Run LocalLM as an MCP *server* over stdio/HTTP, publishing its
workspace, execution, research and memory tools so other clients (Claude
Code, IDE agents) can drive them.

**Why here.** The MCP client half is fully built (`connectors.rs`,
`tool_discovery.rs`, `local_mcp_process.rs`, `local_mcp_config.rs`); the
schemas, policy checks and envelopes already exist. Inverting the transport
turns the app into shared infrastructure rather than a closed workspace.

**Where.** New `mcp_server.rs`; reuse `harness.rs::definition` for schemas
and the existing approval/audit path.

**Effort.** L.

**Watch out.** An external client has no conversation and no user-visible
approval surface — it needs its own explicit, per-client, per-tool consent
model, bound to loopback with an auth token, defaulting to read-only and
off. Do not inherit a conversation's Full-access mode.

### 3.2 OpenAI-compatible local endpoint

**What.** Serve `/v1/chat/completions` (and `/v1/models`) from the app,
backed by the *harness* rather than raw `llama-server`, so external editors
get the agent loop, tools and permissions instead of bare completions.

**Why here.** The OpenAI wire format is already implemented on the client
side (`providers.rs`), so request/response mapping is largely a mirror of
existing code. It makes every model-configuration and prompt-cache
improvement available to other tools.

**Effort.** L.

**Watch out.** Same consent problem as §3.1, plus SPEC.md's rule never to
expose the runtime on the network: bind loopback only, require a token, and
make the binding state visible in the UI.

### 3.3 Browser-rendered page tool

**What.** A headless-browser fetch path for JavaScript-dependent pages,
returning extracted text through the existing evidence pipeline, plus
optional screenshot capture into the vision path.

**Why here.** HARNESS-IMPROVEMENT.md names this as a known, deliberate gap:
JS-dependent pages currently surface as snippet-only evidence with an
explicit limitation string. Playwright is already a devDependency and
`playwright.config.ts` / `e2e/` are already wired.

**Where.** `src/web/fetch/` as a fallback transport,
`harness.rs` (`web_fetch_url` mode flag), `sandbox.rs` for SSRF rules.

**Effort.** M–L.

**Watch out.** A browser is a large new attack surface for hostile page
content and a large install; it must be optional, off by default, subject to
the same SSRF/redirect/robots policy as `web-transport`, and must never be
silently substituted for the plain fetch path.

### 3.4 OCR for scanned documents

**What.** An OCR path so scanned PDFs and images become readable evidence
instead of failing.

**Why here.** Another explicitly recorded gap — `web_open` currently fails
scanned PDFs with `OCR required`. With an mmproj projector already
supported, the vision model itself is one viable backend; a bundled OCR
engine is the other.

**Where.** `src/web/documents/`, `attachments.rs`, `inference.rs` for the
vision route.

**Effort.** M.

**Watch out.** OCR output is low-confidence text. It must be labelled as
OCR-derived in the evidence record so verification and citation scoring can
weigh it accordingly, never presented as extracted text.

### 3.5 Code intelligence index

**What.** A tree-sitter-backed symbol index over the workspace:
`find_definition`, `find_references`, `outline_file`, plus a diagnostics
tool that runs the project's own type-checker/linter and returns structured
errors.

**Why here.** The Coding preset navigates code with fixed-string
`file_search` and full-file reads, which is expensive in tokens and weak for
a small model. Structured navigation cuts both.

**Where.** New `code_index.rs` + harness tools; watcher for incremental
updates; pairs naturally with §1.4 for hybrid lexical+semantic search.

**Effort.** L.

**Watch out.** Index build time and memory on a large repository; make it
incremental, cap it, and degrade to `file_search` with a stated reason
rather than blocking.

### 3.6 Additional sandbox providers

**What.** Extend `sandbox.rs` beyond Docker and raw local execution: WSL2,
Windows Sandbox, Podman, and a network-namespace-less "offline" profile.

**Why here.** `docker_exec` already exists and fails loud without Docker
(`harness.rs`), and Docker is not installed on the primary target machine
(HARNESS-IMPROVEMENT.md baseline). WSL2 is usually present on Windows 11 and
gives a real boundary where local execution gives none.

**Where.** `sandbox.rs::provider`, `execution.rs`, `Execution.tsx`.

**Effort.** M per provider.

**Watch out.** Each provider's actual isolation properties must be stated
precisely in the UI. Do not let a weaker provider inherit a stronger
provider's language — the SPEC forbids representing local processes as an
isolation boundary, and the same honesty applies to each tier.

### 3.7 In-app skill authoring

**What.** Create, edit, validate and locally install skills from inside the
app, with a linter for frontmatter/description quality and a test-run
harness.

**Why here.** 13 pinned skills install today with integrity verification
(`skills.rs`, `catalog/skills.lock.json`) but skills are read-only artifacts
from upstream; `skill-creator` is itself one of the installed packages. A
local authoring loop lets the workspace grow its own procedures.

**Where.** `skills.rs` (local-source install path),
`catalog/skill-catalog.yaml`, `Skills.tsx`.

**Effort.** M.

**Watch out.** Locally authored skills must be marked distinctly from pinned
upstream ones in provenance, and must not gain execution permission by
existing — the README's rule that instructions never confer execution rights
still holds.

### 3.8 Scheduled and event-triggered runs

**What.** Cron-style recurring runs and file-watch triggers ("every weekday
at 09:00 summarise the changed files"), executing an existing conversation
preset in the background.

**Why here.** `runs` / `run_events` / `subagent_runs` already model
background execution with durable settlement; scheduling is a trigger layer
over machinery that exists. There is no cron anywhere in the tree today.

**Where.** New `scheduler.rs`, `agent_run.rs`, tray/notification UI.

**Effort.** M.

**Watch out.** An unattended run cannot answer an approval prompt. It must
run under an explicitly pre-authorised, narrow tool set — never Full access
by default — and `ask_user` must fail the run with a clear reason rather
than hanging.

### 3.9 Desktop integration tools

**What.** Screenshot capture into the vision path, clipboard read/write, and
native notifications on run completion or approval requests.

**Why here.** Background subagents and long research tasks finish silently
today. Screenshot + mmproj makes "look at this error dialog" work without a
file round-trip.

**Where.** Tauri plugins, `attachments.rs`, `harness.rs::registry`.

**Effort.** S–M.

**Watch out.** Screen and clipboard capture are privacy-sensitive: explicit
per-use approval, never a trusted read, and captured content must be handled
as conversation data under the existing privacy statement.

---

## Tier 4 — Operations, UX and evidence

### 4.1 Cost and usage ledger

**What.** Per-run accounting of tokens, wall time and — for remote
providers — cost, aggregated per conversation, preset and model.

**Why here.** Prerequisite for §2.6 to be meaningful, and the only way to
compare local vs. remote honestly. `providers.rs` already knows per-model
context and output limits.

**Effort.** M. **Watch out.** Remote pricing is external data that goes
stale; store it as user-entered configuration and label it as such rather
than shipping guessed rates.

### 4.2 Trajectory diff and annotation

**What.** Compare two trajectories side by side (§2.7), annotate any event
with a note, and export the annotated trace as a reproducible bug report.

**Why here.** `Trajectory.tsx` already filters, searches and inspects raw
JSON, and `export.rs` already exports conversations. This closes the loop
from "the agent misbehaved" to a shareable artifact.

**Effort.** M. **Watch out.** Exports must keep honouring the existing
redaction rules in `docs/EXPORTS.md`.

### 4.3 Workspace-scoped project profiles

**What.** Bind a preset, tool selection, execution interpreters, permission
mode and memory scope to a workspace folder, restored automatically when
that folder is opened.

**Why here.** All of these settings are per-conversation or global today;
in practice they are properties of the project. `memory_facts` already
carries a `scope`.

**Effort.** M. **Watch out.** A profile must never silently raise the
permission mode when a folder is opened — restoring Full access without a
prompt would defeat the approval model.

### 4.4 Crash recovery and interrupted-turn resume

**What.** Detect a turn interrupted by a crash or power loss and offer
resume-from-last-event or discard.

**Why here.** SPEC.md's conversation checklist item explicitly includes
"recover interrupted turns" and it is still open; `session_events` plus
`research_tasks` checkpointing already provide most of the substrate.

**Effort.** M. **Watch out.** Never auto-replay a tool whose outcome is
unknown — mirror the existing `research_cancel` rule that unknown-outcome
operations are not auto-replayed.

### 4.5 Streaming reasoning and tool-call preview

**What.** Render reasoning separation and partial tool-call arguments as
they stream, with a cancel affordance before a tool is dispatched.

**Why here.** Small models spend a long time in reasoning; showing it makes
the wait legible and lets a user kill an obviously wrong plan before it
executes. `sse.rs` and `chat.rs` already parse the stream.

**Effort.** M. **Watch out.** Partial arguments are not a commitment; never
execute from a preview, and make cancellation land before dispatch, not
after.

---

## Suggested sequencing

If the goal is *measurably better agent behaviour on a local 2B–7B model*,
this order compounds:

1. **§2.5 eval harness + §1.5 seeds** — without them, every later claim is
   unverifiable, and PROGRESS.md's evidence standard cannot be met.
2. **§1.1 grammar-constrained decoding** — largest single quality win;
   removes the malformed-tool-call failure class outright.
3. **§1.2 KV slot save/restore + §1.7 telemetry** — largest latency win, and
   the telemetry proves it.
4. **§2.2 checkpoint/rollback + §2.3 git tools** — makes the Coding preset
   safe enough to use with less supervision.
5. **§1.4 embeddings + §3.5 code index** — cuts token cost on navigation and
   recall; both feed §1.1's smaller, better-targeted contexts.
6. **§2.1 hooks + §2.8 plan mode** — user-level control surfaces, best built
   once the loop underneath is trustworthy.
7. **§3.1/§3.2 outward interfaces** — only after the permission story for
   external callers is designed, not retrofitted.

## Deliberately not recommended

- **A second agent loop for any new mode.** `presets.rs` is explicit that
  presets are config patches, not separate executors. Forking the loop
  duplicates the approval and audit path, which is exactly where
  correctness lives.
- **Prompt-only fixes for structured-output problems.** HARNESS-IMPROVEMENT.md
  opens with this: prompts alone never guarantee correctness. Prefer §1.1.
- **Mandatory paid search APIs.** The stated constraint is SearXNG-first with
  no required keys; keep any commercial provider strictly optional.
- **Bundling a full browser engine by default.** §3.3 should stay opt-in;
  the install-size and attack-surface cost is real and the current explicit
  limitation string is an honest fallback.
- **Cloud sync of conversations.** The data model is deliberately local and
  unencrypted-at-rest by the application; syncing it would change the
  privacy contract in README "Data and privacy" and needs a separate design.
- **Auto-approving new tool classes under Auto-approve reads.** The trusted
  read set (`harness.rs::is_trusted_read`) is small on purpose; shell
  execution is never a trusted read, and new tools should default to asking.
