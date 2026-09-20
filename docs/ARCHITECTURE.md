# Architecture: Capability Seam & Append-Only Session Events (DSH Adaptation)

## 1. Capability Microkernel Seam (`src-tauri/src/capabilities.rs`)

LocalLM incorporates DeepSeek Harness's plugin microkernel model (adapted from Cordis `ctx.effect()` and `ctx.on()`) into idiomatic Rust:

- **`CapabilityKind`**: Categorizes capabilities into `Model`, `Tool`, `Skill`, `Session`, `Sandbox`, `Storage`, `Loop`, `Ui`.
- **`Capability`**: Typed record holding `id`, `kind`, `version`, `enabled`, `description`, and `config: serde_json::Value`.
- **`Registry`**: In-memory registry tracking registered capabilities, dependencies (`requires`), lifecycle hooks (`effects`), presets, and defaults.
- **`capabilities.json`**: Static authority declaring the full capability tree, default configurations, and presets (`standard`, `code_ptc`, `minimal`, `creator`).
- **Temporal & Spatial Composability**:
  - Gated capabilities (e.g. `system_time`, `workspace_files`, `local_execution`) can be enabled or disabled globally in SQLite (`capabilities.disabled`) or scoped per-conversation (`conversation_tools`).
  - Disabling a capability detaches prompt sections, tool schemas, and IPC execution paths cleanly.
- **IPC Interface**:
  - `list_capabilities(conversationId?)`: Reports all capabilities with effective enabled status.
  - `set_capability_enabled(id, enabled)`: Validates capability ID, guards by `operation` mutex, audits decision, and updates persistence.
  - `dump_config(conversationId?)`: Read-only dump of active preset, capability states, configuration trees, and tool bindings without secrets.

## 1b. Prefix-stable prompt layout (`src-tauri/src/prompt.rs`)

Local generations send `cache_prompt: true` to llama.cpp. Hits require a byte-identical token prefix:

- One frozen system message (preferences, skills, response style, CODE PTC SDK).
- Frozen tool schemas from `conversation_prompt_freeze` (replaced only when the user changes Tools/skills/preset).
- Exact history replay from `messages` audits (`callId`, `stepId`, `assistantContent`); `session_events` remains the Trajectory log, not the send-path reconstruction.
- Trailing volatile system (memory, plan, unavailable tools) then the draft.

Parent requests pin `id_slot: 0`. Subagents use slot 1 when Runtime **Inference slots** is 2 (`--parallel 2`). Token counts strip `cache_prompt` and `id_slot`.

## 2. Append-Only Session Events (`src-tauri/src/store.rs`)

Session persistence is modeled as an immutable, monotonic event log (`session_events` table):

- **Authority**: Format version governed by `SESSION_FORMAT_VERSION = 1` and `docs/session-format-status.md`.
- **Schema**:
  - `id`: Unique event ID (`evt-<uuid>`).
  - `conversation_id`: Associated conversation.
  - `run_id`: Associated run lifecycle execution (optional).
  - `seq`: Monotonically increasing sequence number per conversation.
  - `step_id`: Agent loop step identifier (e.g. `step-0`).
  - `tool_call_id`: Deterministic tool call ID if event is tool-scoped.
  - `event_type`: Typed event identifier (`system_prompt`, `context_injection`, `user_msg`, `turn_start`, `turn_end`, `step_start`, `step_end`, `reasoning`, `tool_call`, `tool_result`, `subagent_schedule`).
  - `payload`: JSON payload storing the exact model-visible text, tool call parameters, or execution outcomes.
  - `ignorable`: Boolean flag for replay reconstruction.
  - `created_at`: Unix timestamp in milliseconds.
- **Model-Visible ⟺ Logged Invariant**: Any prompt, reasoning token, tool call, or injected context sent to or emitted by the model is durably committed to `session_events`.

## 3. Session Branching (Forking) & Replay

- **Lineage Preservation**: Forking from sequence `N` clones events up to `N` into a new conversation with new IDs and links `forked_from_conversation_id` and `forked_from_seq`.
- **Replay**: Re-derives model conversation history from the durable event log.

## 4. Trajectory UI (`src/components/Trajectory.tsx`)

- Filter by source (All, System Prompts, Injections, User Messages, Reasoning, Tool Calls, Tool Results).
- Search query filtering across event payloads and types.
- Detail inspector drawer for raw JSON inspection with copy-to-clipboard.
- "Branch from here" button allowing users to fork a conversation at any historic step.
