# Why the latest agent run stopped

9 November 2026. Evidence is the live app database
`~/.local/share/app.locallm.desktop/locallm.sqlite` and the current source.
Labels are **harness** or **model**.

## Latest run — harness round cap, not a crash

Conversation `5c74c45b-74b2-4e5c-b661-5ed2a5b103fa`, DeepSeek-V4.1-Flash,
workspace `/home/papan/Desktop/Projects/omen`, goal
`PRODUCTION_READINESS_PROMPT.md`.

| Fact | Value |
| --- | --- |
| Wall time | 381.4 s (14:06:02Z–14:12:23Z) |
| Model rounds | 64 (`step_start` 0–63) |
| Saved budgets | `taskTimeoutSecs` 1800, `maxRounds` 64, `operationTimeoutSecs` 300 |
| Turn end | `stopReason=budget_exhausted`, `partial=true`, `delivered=true`, `error=null` |
| Run row | `paused`, checkpoint `step-63` |
| Task row | `budget_exhausted`, resume id `task_8c0ebc54140042c3877b3822eafbf09d` |
| Goal gates | G1–G21 still `pending` |
| Last-round cache | rounds 1–62 reused the prefix (round 62: 83,968 of 85,849). Round 63 cache is 0 because tools were removed for the final answer. |

The model did not die. It was told the budget was gone and wrote a status
report. The 30-minute task clock and the 10-minute model deadline never fired.
A fast model burns 64 rounds in minutes.

**Harness.** The parent loop is a hard `for round in 0..max_rounds` and then
returns. It does not open another segment.

- Cap comes from `continuation::Budgets::default` (`max_rounds: 64`) via
  `TaskBudgets::for_parent` and `Budgets::for_task`.
  `src-tauri/src/continuation.rs:111`, `src-tauri/src/research_tasks.rs:126`,
  `src-tauri/src/chat.rs:1931`, `src-tauri/src/chat.rs:1958`.
- With about 20 seconds of the segment left, or when
  `rounds_used + 1 >= max_rounds`, the loop injects a no-tool final answer and
  sets `partial_work`. `src-tauri/src/chat.rs:1982-1990`,
  `src-tauri/src/continuation.rs:144`.
- After the loop, a partial answer is saved and the run is paused because goal
  gates are still pending. Nothing schedules the next segment.
  `src-tauri/src/chat.rs:3503-3507`, `src-tauri/src/chat.rs:4924-4945`.
- Resume exists only as an explicit user action (`resume_task_id`). There is
  no autonomous continuation after `budget_exhausted`.

**Model.** The model spent the rounds on real edits and checks, then correctly
refused to claim the release was done. It did not update the goal ledger, so
all 21 gates stayed pending. That is model behavior on top of the harness cap,
not the reason the loop ended.

## Same goal, earlier today — same cap, worse ledger mismatch

Conversation `03549ce7-ecba-481c-8181-97d5a5d1485a`, 09:34:08Z–09:39:32Z
(324 s). Also 64 `step_start` events. Task status `budget_exhausted`, but the
run row is `completed` and the saved budget says `maxRounds: 8` (the
`TaskBudgets` default, not the loop cap).

**Harness.** The event log and the persisted budget disagree. Current source
writes the loop cap through `for_parent`, and the 14:06 run saved `maxRounds:
64`, so that split is not what the binary that just ran does. The 09:34 row
is retained evidence of a ledger/loop mismatch; this session did not recover
the binary that wrote it.

`ResearchTask::budget_exhausted` still counts every operation as a round
(`operations.len() >= max_rounds`) and only counts `kind == "search"`.
`src-tauri/src/research_tasks.rs:268-275`. The loop ignores that return value
(`src-tauri/src/chat.rs:3709`). It did not end this run. It can still mark a
task exhausted for the wrong reason.

## Tool alignment — harness, and it burned rounds

These calls failed before they did the work the model asked for. 76 tool
calls, 8 problem ledger rows.

| What the model sent | What the harness did | Where |
| --- | --- | --- |
| `timeout_seconds` above the saved shell lifetime | Rejected: "The configured process lifetime is 60 seconds." Schema advertises a maximum of 3600 and says the lifetime is the configured timeout. Default configured timeout is 60. The model cannot raise it. | `src-tauri/src/process_tools.rs:28`, `src-tauri/src/process_tools.rs:403-405`, `src-tauri/src/execution.rs:41-46` |
| A cargo/build command that outlived 60 s | Process tree killed: "Configured process timeout exceeded." This is the shell lifetime, not the 1800 s task budget or the 600 s model deadline. | `src-tauri/src/process_tools.rs:878-901` |
| `yield_ms` above 1000 | Rejected. `process_poll.wait_ms` is also capped at 1000, while `check_report.wait_ms` allows 30000. | `src-tauri/src/process_tools.rs:29-35`, `src-tauri/src/process_tools.rs:400`, `src-tauri/src/check_tools.rs:835` |
| `apply_patch` changes that were not the advertised `oneOf` array, or a replacement the patcher refused | Rejected before the edit. Schema is `src-tauri/src/workspace_patch.rs:1832`. | model-shaped arguments, harness rejection |
| `shell_run` schema field `timeout_secs` (max 600, "wait, then return the live id") | Different name and meaning from `timeout_seconds` on `process_start` / `check_run`. | `src-tauri/src/process_tools.rs:41-44` |

Nonzero exits and the killed process were stored as `unknown` because a
mutating tool with `sideEffects: unknown` is not a clean failure.
`src-tauri/src/chat.rs:3768-3775`. The handoff then says "review before
retrying" for calls whose outcome is a validation error or a killed tree, not
an uncertain write. That is harness classification, and it makes the next
segment less able to continue.

The ipc-test failure after the model's own edit is **model**. The harness
reported it (`check_report` / `checks-command_failed`) and then ended the
segment before the model could fix it.

## Timeouts that did not end this run

| Clock | Limit | This run |
| --- | --- | --- |
| Segment | 1800 s (`research_tasks.rs:92`, `chat.rs:1928`) | 381 s used |
| Model HTTP | 600 s (`chat.rs:2063`) | longest round 8.0 s |
| Stream wait vs segment deadline | `chat.rs:2062`, `chat.rs:2095` | not hit |
| Shell lifetime | configured 60 s | hit once; the loop continued |
| Provider `inference_timeout` | upstream, ~125 s on older coffee-site runs in the same database | not this turn |

Older failed runs (`inference_timeout`, `error decoding response body`) are
**model/provider**. The harness records them and ends that turn. They are a
different defect from the round cap.

## The 2,652 s soak is not a harness exit

Every retained soak log that contains `elapsedSeconds: 2652` continues. That
line is a progress sample printed about every 30 seconds while the fixture
waits. `src-tauri/src/harness_endurance_tests.rs:128-137`.

The note in `docs/HARNESS_ACCEPTANCE_REMEDIATION_2026-11-09.md:71` says an
earlier attempt stopped near 2,652 s and left no one-hour completion. No log,
error, or run row for that attempt was retained. Later one-hour and two-hour
soaks completed (`docs/HARNESS_ACCEPTANCE_REVALIDATION_2026-11-09.md:87`).
**Not a demonstrated harness timeout.** Do not treat 2,652 as a product deadline.

## What is not the cause

- System prompt. One `system_prompt` event, about 2.3 KB. Prefix cache held
  through round 62. The prompt did not end the run.
- Compaction. No compaction event on this turn. Input grew to 85,849 tokens
  inside a 1,048,576 context. Context overflow did not end the run.
- A model crash or an empty generation. 64 complete rounds, a visible answer,
  `error=null`.

## Fix order, from this evidence

1. **Harness.** After a goal segment hits the round or time cap, start the
   next segment from the saved task instead of waiting for a click. Keep the
   per-segment cap. This is the stop the user hit.
2. **Harness.** Advertise the timeout the runtime will accept, or accept a
   longer lifetime up to the already-documented 3600 s ceiling when the user
   has allowed it. Stop killing `cargo test` at the 60 s default while the
   schema says 3600.
3. **Harness.** One timeout field name and one wait-ms ceiling across
   `shell_run`, `process_start`, `process_poll`, and `check_run`.
4. **Harness.** A rejected call that never started is `failed`, not `unknown`.
5. **Not this bug.** Provider `inference_timeout` and model edit mistakes.
