# Session format status (authority)

`SESSION_FORMAT_VERSION = 1` (`src-tauri/src/sessions.rs`).

## What the model sees is what the log holds

- `messages`: user/assistant/tool rows (audit JSON for tools).
- `runs` + `run_events`: per-turn states plus `step_start`, `context_injection`,
  `plan`, `subagent_schedule`, `subagent_interrupt`, `workflow`, `ralph`,
  `ptc`, `fork`, `compaction` events.
- `subagent_runs`: parent/child lineage with depth, status, label.
- `todos` + `goals`: plan state injected every turn, mutated only via tools.
- `compaction`: cutoff + checkpoint artifact id; history rebuild drops the
  prefix and inserts a citing notice. Audits are never deleted.
- `memory_facts`: taught facts; recall injects verbatim blocks.

## Operations

- Fork copies message rows with fresh ids plus a `fork` lineage event.
- Replay re-derives the transcript deterministically over stored rows; a live
  re-run is non-deterministic (model sampling) and is not claimed otherwise.
- Search is a bounded LIKE over message content (100 rows max).

## Non-goals (v1)

No cross-device sync, no event encryption at rest beyond OS storage, no
retroactive rewrite of committed generations (successor versions only).
