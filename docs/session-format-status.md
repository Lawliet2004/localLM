# Session Format Status & Invariants Authority

## 1. Specification

- **Current Format Version**: `SESSION_FORMAT_VERSION = 1`
- **Persistence Engine**: SQLite (`locallm.sqlite`), `session_events` table.
- **Data Model**: Append-only, monotonically sequenced event log per conversation.

## 2. Table Schema

```sql
CREATE TABLE IF NOT EXISTS session_events (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
    seq INTEGER NOT NULL,
    step_id TEXT,
    tool_call_id TEXT,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    ignorable INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS session_events_conv_seq ON session_events(conversation_id, seq);
CREATE INDEX IF NOT EXISTS session_events_run ON session_events(run_id);
CREATE INDEX IF NOT EXISTS session_events_type ON session_events(event_type);
```

## 3. Event Types & Invariants

| Event Type | Producer | Content / Payload | Replay Semantics |
|---|---|---|---|
| `system_prompt` | `chat.rs` | Baseline system prompt + active skill instructions | Replay source for system state |
| `context_injection` | `chat.rs` | Telemetry, system time, or workspace state | Informational / audit |
| `user_msg` | `chat.rs` | User message text and attachment references | User turn input |
| `turn_start` | `chat.rs` | Turn metadata (model selection, access mode) | Turn boundary |
| `step_start` | `chat.rs` | Agent round index (0..8) | Round boundary |
| `reasoning` | `chat.rs` | Thinking / chain-of-thought text | Non-replayed during history reconstruction |
| `tool_call` | `chat.rs` | Tool alias, original name, arguments JSON | Reconstituted as assistant tool_calls |
| `tool_result` | `chat.rs` | Execution output, stdout/stderr, or denial message | Reconstituted as tool role message |
| `step_end` | `chat.rs` | Step completion status | Step lifecycle |
| `turn_end` | `chat.rs` | Turn outcome (`completed`, `cancelled`, `error`) | Turn boundary |
| `subagent_schedule` | `subagents.rs` | Subagent dispatch parameters | Subagent delegation |

## 4. Guarantees

1. **Model-Visible ⟺ Logged**:
   - Anything sent to the model (system prompt, history, tool results, injections) or received from the model (tokens, reasoning, tool calls) must produce a committed row in `session_events`.
   - Send-path history replay uses `messages` tool audits (`callId`, `stepId`, `assistantContent`). `session_events` is the Trajectory/audit log, not a second prompt reconstruction engine.
2. **Monotonic Sequences**:
   - For any given `conversation_id`, `seq` starts at 1 and increases monotonically without gaps.
3. **Non-Destructive Migrations**:
   - Historical tables (`conversations`, `messages`, `runs`, `run_events`) are strictly preserved.
   - Historical messages and runs are backfilled into `session_events` upon upgrading to `SESSION_FORMAT_VERSION = 1`.
4. **Lineage Preservation on Fork**:
   - Forking at sequence `N` creates a new conversation containing events `1..=N` with new UUID event IDs, setting `forked_from_conversation_id` and `forked_from_seq` metadata.
