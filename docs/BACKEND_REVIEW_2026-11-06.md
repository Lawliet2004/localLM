# Expanded backend review — 6 November 2026

LocalLM has reproducible backend defects that can lose model-visible information, misreport completion, truncate command output, and execute hooks in the wrong workspace. These are separate from the saved upstream inference timeouts. Improving the backend will make tasks more reliable, but cannot guarantee a provider's generation speed or a model's correctness.

This report describes the current working tree, including the earlier inference/artifact fixes. Findings below remain unfixed unless explicitly stated otherwise. This expanded review changed documentation and disposable audit fixtures only; it did not apply another batch of production fixes.

## Scope and evidence

The backend inventory contains 136 Rust source files, approximately 109,000 lines including tests. Review covered the module inventory and traced the important flows across inference adapters and streaming, tool discovery and validation, dispatch and authorization, workspace reads/writes, process execution, hooks and formatting, history and compaction, sessions and persistence, MCP input/transport, local runtime configuration, and verification. Adjacent frontend invocation and status handling were checked where they affect these flows. The inventory is not a claim that every frontend, test, or generated line received the same depth of inspection.

Evidence is distinguished throughout:

- **Reproduced:** 14 isolated checks exercised the existing implementation and demonstrated problems in history, hooks, formatting, processes and snapshot confinement.
- **Source confirmed:** a concrete control-flow or contract mismatch is present, but the corresponding storage failure, real account, long interactive wait, or GPU/runtime scenario was not exercised end to end.
- **Improvement:** a design change that would improve reliability or performance, rather than an assertion that an existing guarantee was violated.

The audit fixtures use temporary databases and workspaces. No paid live model request, real-account refresh, production database write, browser preview, or GPU inference was performed. The saved application database was inspected read-only during the earlier investigation.

Companion evidence:

- [Reproduction output](/home/papan/Desktop/Projects/localLM/docs/backend-review-2026-11-06/probe-results.txt).
- [Audit-only reproduction patch](/home/papan/Desktop/Projects/localLM/docs/backend-review-2026-11-06/repro.patch).
- [Source inventory and hashes](/home/papan/Desktop/Projects/localLM/docs/backend-review-2026-11-06/source-inventory.csv).
- [How to reproduce](/home/papan/Desktop/Projects/localLM/docs/backend-review-2026-11-06/README.md).
- [Earlier inference investigation and fixes](/home/papan/Desktop/Projects/localLM/docs/INFERENCE_FAILURE_AUDIT_2026-11-06.md).

Priorities: **P1** should be addressed first because information, execution scope, or task truth can be wrong. **P2** should follow because specific workflows fail, stall, or incur avoidable latency. Priority is conditional on using the affected feature; a hooks-only defect does not explain a run with no configured hooks.

## Findings

### B01 · P1 · Send-time compaction does not save the archive it first promises

**Reproduced; source:** [chat.rs:4250](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:4250), [chat.rs:4269](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:4269), [compaction.rs:272](/home/papan/Desktop/Projects/localLM/src-tauri/src/compaction.rs:272).

The budget loop plans compaction from the original messages and counts the candidate. It then passes the candidate's already reduced `kept` messages into `compact_model_messages`, which plans again. The first plan's dropped messages and archive ID are never persisted. The second archive can contain already reduced history and a notice referring to the missing first archive.

The fixture used five older turns with unique text beyond the short excerpts. The first archive did not exist, and the replacement archive did not contain the original discarded text. The underlying conversation rows remain in SQLite; the broken guarantee is that `artifact_read` can retrieve the verbatim history promised to the model. The token count also describes the first candidate rather than necessarily the final committed payload.

**Repair:** separate planning from committing. Persist the exact accepted plan's `dropped` messages and ID, install that plan's `kept` messages, and count the payload that will actually be sent. Check that every compaction notice resolves to a conversation-owned archive after both immediate continuation and reload.

### B02 · P1 · Compaction drops history even when it fits the configured retained window

**Reproduced; source:** [compaction.rs:192](/home/papan/Desktop/Projects/localLM/src-tauri/src/compaction.rs:192), [compaction.rs:219](/home/papan/Desktop/Projects/localLM/src-tauri/src/compaction.rs:219).

`retention_start = None` is documented to mean that all historical turns fit and nothing needs dropping. The retention predicate instead retains historical messages only when `retention_start` is `Some`. With one previous user/assistant turn and `keep = 10`, the planner dropped both previous messages, including a requirement explicitly marked for preservation.

This occurs when compaction is invoked; ordinary uncompacted turns are not all losing their history. It also makes B01's second planning pass particularly damaging.

**Repair:** represent “retain all history” explicitly, preserve complete tool exchanges, and cover zero, fewer than, equal to, and more than the configured number of historical turns.

### B03 · P1 · Snapshot restore can write outside the workspace through a replaced parent directory

**Reproduced; source:** [snapshots.rs:795](/home/papan/Desktop/Projects/localLM/src-tauri/src/snapshots.rs:795), [snapshots.rs:886](/home/papan/Desktop/Projects/localLM/src-tauri/src/snapshots.rs:886).

Capture walks parents with no-follow capability handles. Restore instead constructs `root.join(path)` and checks only the leaf with `symlink_metadata`; that still follows symlinks in parent components. Its temporary write and rename use ordinary path-based filesystem operations.

The fixture captured `nested/file.txt`, staged a revert, replaced `nested` with a link to a separate temporary directory, and committed. The outside file was overwritten with the captured contents, and the report contained zero failed paths. A leaf-link test already exists, but does not cover this parent-link case. Deletion of files recorded as absent also uses a path-based remove through the parent; the overwrite case was the one reproduced.

**Repair:** restore and delete through scoped, no-follow parent handles, as workspace transactions already do. Hold the shared workspace transaction lock and preserve external changes when identity cannot be established. Cover parent replacement for both write and delete restores.

### B04 · P1 · Hooks execute in the global workspace instead of the conversation's project

**Reproduced; source:** [hooks.rs:163](/home/papan/Desktop/Projects/localLM/src-tauri/src/hooks.rs:163), [hooks.rs:175](/home/papan/Desktop/Projects/localLM/src-tauri/src/hooks.rs:175).

`pre_use`, `post_use`, and run-finished hooks have a conversation ID, but `hook_context` selects `store.workspace_path()` without resolving that conversation's project/worktree. Tools themselves use the conversation's execution snapshot. A hook that formats, tests, or writes relative paths can therefore operate on a different project than the tool it accompanies.

The fixture assigned a conversation to temporary project B while the global workspace was A. A `pwd` hook wrote its marker in A, even though `task_workspace` resolved B.

**Repair:** bind hooks to the same canonical workspace snapshot as the dispatch they accompany; include that identity in the event. Define run-finished behavior when a worktree is no longer available rather than falling back to another project.

### B05 · P1 · Hook timeout neither stops the process tree nor covers stdin transmission

**Reproduced in two checks; source:** [hooks.rs:141](/home/papan/Desktop/Projects/localLM/src-tauri/src/hooks.rs:141), [hooks.rs:145](/home/papan/Desktop/Projects/localLM/src-tauri/src/hooks.rs:145), [chat.rs:3396](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:3396).

Hooks use raw `Command::spawn` without the managed process-tree owner used elsewhere. Timing out `wait_with_output` drops its future without killing the command. A one-second hook timeout was followed by a file write from the still-running hook after two seconds.

The timeout also starts only after writing and shutting down stdin. A hook that does not consume a large event can block before that deadline exists. A fixture configured for one second instead returned success after about two seconds because its stdin write waited for the child to exit. Output collection is unbounded as well. Foreground post-hook waiting does not wrap the call in the cancellation select used for pre-hooks.

**Repair:** use managed process-tree ownership, one absolute deadline covering input/output/exit, bounded drains, and explicit cancellation cleanup. Report uncertain side effects when interruption occurs; do not rerun the original operation automatically.

### B06 · P1 · Forking at an event can include messages from after the fork point

**Reproduced; source:** [store.rs:2078](/home/papan/Desktop/Projects/localLM/src-tauri/src/store.rs:2078), [store.rs:2086](/home/papan/Desktop/Projects/localLM/src-tauri/src/store.rs:2086).

Forking filters journal events by sequence, but copies messages using `min(copied_event_count, source_message_count)`. Events and messages do not advance one-for-one: one response can emit many tool, reasoning, approval, and progress events. This is not a valid mapping from event sequence to message boundary.

The fixture forked at event four and copied both a later user request and its later answer. The fork also copies current todos and goals rather than reconstructing their state at the selected boundary. This can give the model future instructions or a completed answer from work the fork should not contain.

**Repair:** persist explicit event-to-message/turn associations and branch at a consistent boundary. Reconstruct messages, task state, tool audit, and native context from that boundary, with correct ID remapping. Do not infer coverage from event counts or timestamps.

### B07 · P1 · File-write contracts advertise inputs the model-call assembler rejects

**Reproduced in the earlier review; source:** [tool_calls.rs:154](/home/papan/Desktop/Projects/localLM/src-tauri/src/tool_calls.rs:154), [tool_calls.rs:178](/home/papan/Desktop/Projects/localLM/src-tauri/src/tool_calls.rs:178), [workspace_patch.rs:1838](/home/papan/Desktop/Projects/localLM/src-tauri/src/workspace_patch.rs:1838).

`write_file` advertises up to 1 MiB of content and the binary variant allows 1.4 million base64 characters. Native model-call assembly rejects the entire serialized arguments string above 65,536 bytes. JSON syntax, escaping, path and hash fields all consume that smaller allowance. A 65,536-byte content value and a 70,000-byte content value were both rejected by the actual assembler.

This is an application contract mismatch and can affect generated pages, scripts, patches, and multi-edit batches. It is separate from the historical upstream timeout failures.

**Repair:** define per-tool transport budgets at the contract layer and make every invocation path enforce them consistently. Offer bounded incremental writes or artifact-backed content for larger files. Reject incomplete arguments before dispatch; keep protective size limits.

### B08 · P1 · A completed response automatically certifies all open todos as completed

**Source confirmed; source:** [chat.rs:4658](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:4658).

Every `Ok(true)` result marks all pending/in-progress todos completed, regardless of whether individual items succeeded. This includes final answers describing partial work after a budget/checklist limit. Finishing a response is not evidence that every requested action was performed.

**Repair:** retain unfinished todo state or assign explicit partial/blocked/unverified outcomes. Let actual task transitions close tasks. Response delivery and task completion should remain separate facts. This does not require treating every successfully delivered answer as a failed generation.

### B09 · P2 · `shell_run` discards unread output when the process exits

**Reproduced; source:** [process_tools.rs:589](/home/papan/Desktop/Projects/localLM/src-tauri/src/process_tools.rs:589), [process_tools.rs:612](/home/papan/Desktop/Projects/localLM/src-tauri/src/process_tools.rs:612).

The adapter reads one 65,536-byte page and breaks as soon as the process state is no longer `running`, ignoring `hasMore`. A fast command produced 150,000 bytes, all retained in the process buffer, but `shell_run` returned 65,536 with `truncated = false` and a successful exit. The missing bytes can contain the failure summary or useful file data. They remain recoverable through the process registry; the returned shell result does not disclose the loss or its continuation cursor.

**Repair:** drain remaining pages after exit up to the overall output allowance. If output remains, return an explicit truncation flag and usable cursor/handle. Bound total collection without falsely presenting a complete result.

### B10 · P2 · Polling sleeps even when output and exit are already available

**Reproduced; source:** [process_tools.rs:518](/home/papan/Desktop/Projects/localLM/src-tauri/src/process_tools.rs:518), [process_tools.rs:527](/home/papan/Desktop/Projects/localLM/src-tauri/src/process_tools.rs:527).

`process_poll(wait_ms)` always sleeps before taking a snapshot. `shell_run` requests a one-second wait, so even `true` took about 1,006 ms in the fixture. This delay can accumulate across many small shell calls and is independent of model latency.

**Repair:** first check for new output or a terminal state, then wait on a notification only when necessary. Use a race-safe notification pattern, and retain the bounded wait as a maximum rather than a mandatory delay.

### B11 · P2 · A valid small process-output page can make no cursor progress

**Reproduced; source:** [process_tools.rs:160](/home/papan/Desktop/Projects/localLM/src-tauri/src/process_tools.rs:160), [process_tools.rs:33](/home/papan/Desktop/Projects/localLM/src-tauri/src/process_tools.rs:33).

One raw 4,096-byte read containing invalid UTF-8 can expand to 12,288 bytes after replacement decoding. `process_poll` permits an 8,192-byte page, but refuses a whole frame larger than that limit. It then returns zero frames, the same cursor, and `hasMore = true` indefinitely for the same requested page size.

**Repair:** make every nonempty page advance, either by splitting decoded frames at character boundaries with an appropriate cursor or by selecting a documented frame allowance. Do not silently skip oversized output.

### B12 · P2 · Formatter pipe draining can outlive the advertised hard deadline

**Reproduced; source:** [formatter.rs:429](/home/papan/Desktop/Projects/localLM/src-tauri/src/formatter.rs:429), [formatter.rs:486](/home/papan/Desktop/Projects/localLM/src-tauri/src/formatter.rs:486).

After the formatter leader exits, the function joins stdout/stderr reader threads without a deadline. A descendant can keep those pipes open. A fixture with a 100 ms timeout took about 2,005 ms because a child inherited the pipes. The kill path also kills the leader rather than owning its process tree. Dropping an async `spawn_blocking` await does not stop that blocking work.

**Repair:** use the shared managed process service and one deadline for execution and draining; stop descendants and settle readers on timeout/cancellation. Preserve truthful “may have changed” outcomes for in-place formatters.

### B13 · P2 · Formatter failures become successful tool envelopes

**Reproduced at the normalization boundary; source:** [formatter.rs:719](/home/papan/Desktop/Projects/localLM/src-tauri/src/formatter.rs:719), [formatter.rs:727](/home/papan/Desktop/Projects/localLM/src-tauri/src/formatter.rs:727), [harness.rs:729](/home/papan/Desktop/Projects/localLM/src-tauri/src/harness.rs:729).

The formatter returns `Ok({formatted, skipped, failed})` even when its `failed` array contains errors. The output has no typed failure marker, so shared normalization produces `status = success` and `error = null`. The detail is available in the payload if it survives bounding, but generic status, continuation and UI consumers cannot reliably distinguish all-failed formatting from success.

**Repair:** give the formatter adapter an explicit failure/partial outcome with side-effect certainty, update its output contract, and normalize at the existing shared boundary. Avoid special-case string sniffing in UI consumers.

### B14 · P2 · Forked artifact references remain owned by the source conversation

**Reproduced; source:** [store.rs:2056](/home/papan/Desktop/Projects/localLM/src-tauri/src/store.rs:2056), [harness.rs:888](/home/papan/Desktop/Projects/localLM/src-tauri/src/harness.rs:888).

Forking copies messages/events containing artifact IDs but neither copies their records nor rebinds references. `artifact_read` correctly enforces ownership, so the new conversation cannot read those referenced source-owned artifacts. The fixture confirmed an unchanged artifact ID in the fork with its owner still set to the source conversation.

**Repair:** copy/rebind reachable artifacts as part of the atomic branch operation and rewrite references. Preserve the ownership check. Include compacted history and image artifacts in the branch contract rather than making arbitrary cross-conversation artifacts readable.

### B15 · P2 · Replay and fork silently stop at 5,000 journal events

**Reproduced; source:** [sessions.rs:97](/home/papan/Desktop/Projects/localLM/src-tauri/src/sessions.rs:97), [store.rs:1938](/home/papan/Desktop/Projects/localLM/src-tauri/src/store.rs:1938), [store.rs:2015](/home/papan/Desktop/Projects/localLM/src-tauri/src/store.rs:2015).

Both callers request 10,000 events once, while `session_events` clamps the request to 5,000. Neither caller paginates. A 5,005-event fixture returned/copied only 5,000 without exposing a continuation. Frequent reasoning delta events make this reachable without thousands of user turns. The ordinary trajectory view has its own paging; this finding is specific to the replay/fork APIs.

**Repair:** page internally through a consistent cutoff, or return an explicit paginated contract that callers actually follow. Verify a fork beyond the first page and replay reconstruction across page boundaries.

### B16 · P2 · Large file results lose the file hash needed for safe subsequent edits

**Saved result and source confirmed in the earlier review; source:** [artifacts.rs:786](/home/papan/Desktop/Projects/localLM/src-tauri/src/artifacts.rs:786), [workspace.rs:713](/home/papan/Desktop/Projects/localLM/src-tauri/src/workspace.rs:713).

Generic bounding preserves neither the original file path nor its `sha256`/line metadata. `_sha256` instead identifies the serialized result artifact. Small file reads retain the file hash; the saved bounded read did not. Editing then needs another metadata/artifact round trip, or the model may use the wrong checksum and receive an avoidable stale-hash rejection.

**Repair:** retain a small file-identity/navigation record alongside the excerpt, and distinguish file hashes from artifact checksums. Cover canonical envelopes and compatibility fields without recreating duplicated large content.

### B17 · P2 · Artifact save failures can produce unreadable success pointers

**Source confirmed; source:** [chat.rs:3392](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:3392), [chat.rs:4048](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:4048).

Foreground and prefetched result handling discard `save_artifact` failures. The bounded result can still be sent/persisted with the newly generated artifact ID. If the larger artifact save fails while a smaller audit-row write succeeds, the promised full output is unavailable. No such storage failure was observed in the saved runs.

**Repair:** propagate capture failure or produce a typed, explicit “full output could not be saved” result while preserving the original operation's known outcome and lifecycle handles. Do not replay an already executed write to recover its output.

### B18 · P2 · MCP's interactive form budget exceeds its enclosing tool/HTTP budgets

**Source confirmed; source:** [mcp_input.rs:54](/home/papan/Desktop/Projects/localLM/src-tauri/src/mcp_input.rs:54), [connectors.rs:384](/home/papan/Desktop/Projects/localLM/src-tauri/src/connectors.rs:384), [connectors.rs:1014](/home/papan/Desktop/Projects/localLM/src-tauri/src/connectors.rs:1014), [chat.rs:3080](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:3080).

The form handler permits 600 seconds for a user answer, but an MCP tool defaults to a 120-second enclosing deadline. Saved guard settings can only reduce that default. Remote connector requests also have a 60-second total HTTP deadline. A lengthy interactive tool cannot use the full advertised form budget; transport reconnection does not remove the outer tool deadline.

**Repair:** separate execution time, bounded human-input waiting, connect time, and stream idle time. Carry one explicit cancellation/ownership policy through these layers and expose when a form has expired. Preserve bounded retries and unknown-write outcomes. This review did not spend two or ten minutes on a live remote interactive call.

### B19 · P2 · Local context budgeting assumes configuration equals per-slot capacity

**Source and pinned-upstream compatibility confirmed; no live GPU reproduction. Source:** [runtime_config.rs:103](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime_config.rs:103), [runtime_config.rs:130](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime_config.rs:130), [runtime.rs:272](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime.rs:272), [runtime_config.rs:191](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime_config.rs:191).

LocalLM passes the configured context unchanged as `--ctx-size`, explicitly passes one or two parallel slots, and then reports that configured context to chat after readiness. It does not obtain the actual per-slot context. With split KV allocation, the configured total is divided among slots: a total 32,768-token context with two slots provides approximately 16,384 per sequence, while LocalLM budgets against 32,768. The KV estimate conversely multiplies the configured context by the slot count.

The packaged runtime is pinned to b10855. Its [argument definitions](https://github.com/ggml-org/llama.cpp/blob/b10855/common/arg.cpp#L1621-L1628) distinguish automatic-slot unified KV behavior, and its [context implementation](https://github.com/ggml-org/llama.cpp/blob/b10855/src/llama-context.cpp#L258-L272) divides total context by maximum sequences when KV is split. Behavior depends on the runtime build/allocation mode; a one-slot configuration does not exhibit the two-slot division.

**Repair:** define whether the UI setting is per sequence or total allocation, launch consistently, query runtime properties for effective capacity, and use that value for preflight/send-time budgets. Reconcile the memory estimate with the same definition. Test one and two slots against the supported runtime, including fit-adjusted settings.

### B20 · P2 · Subscription refresh can return an expired token and lose refreshed credentials

**Source confirmed; no real-account request. Source:** [subscription_auth.rs:345](/home/papan/Desktop/Projects/localLM/src-tauri/src/subscription_auth.rs:345), [subscription_auth.rs:392](/home/papan/Desktop/Projects/localLM/src-tauri/src/subscription_auth.rs:392), [subscription_auth.rs:428](/home/papan/Desktop/Projects/localLM/src-tauri/src/subscription_auth.rs:428), [subscription_auth.rs:437](/home/papan/Desktop/Projects/localLM/src-tauri/src/subscription_auth.rs:437).

Non-success refresh HTTP responses fall through to `Ok(session)`, so `get_valid_access_token` can return the same expired access token while hiding the refresh failure. Successful refreshes ignore vault-save failures. Concurrent foreground/child requests can also load and refresh the same session without the refresh gate used by the MCP OAuth credential store.

This affects account-subscription providers, not the River API-key failures already recorded. Ignoring persistence is particularly problematic when refreshed credentials replace the old ones: the current request may work while the next reload has only stale credentials.

**Repair:** serialize refresh per account, re-read under that gate, require a valid refresh response, persist before returning replacement credentials, and expose redacted authentication failures. Exercise these cases through an injected local auth transport rather than a real account.

## Improvements that should accompany the fixes

### Resume a failed model round from durable tool results

The screenshot's failed turn had already completed five tools. “Retry last prompt” submits a new message; it is not a checkpoint-aware continuation of the unfinished model round. The existing paused-research resume mechanism does not cover every failed inference stream.

A recovery operation should preserve completed calls, discard incomplete tool arguments, identify uncertain operations, and retry only the unfinished generation with a small explicit attempt budget. Do not introduce blanket automatic retries for commands or writes. Streamed successful HTTP responses are currently not blindly replayed; that protection should remain.

### Separate task completion, delivery, and verification

The current verification nudge is intentionally advisory and fires once. Another final response can be accepted without a completed relevant check. Together with B08 and B13, that can make the visible result sound more conclusive than the evidence.

Keep explicit outcome fields such as delivered, partial, verified and unverified, with the actual check evidence and unfinished tasks. Apply a verification policy appropriate to the task, respect disabled tools and the declined browser preview, and allow an honest partial answer when required work cannot finish.

### Reduce model round trips before optimizing ordinary file I/O

The historical screenshot run spent about 0.241 seconds preparing the request and roughly 0.01 seconds executing its five tools. Its first model token arrived at 6.516 seconds, and the final model round failed after 100.859 seconds. Four other saved failures explicitly contained upstream `inference_timeout` errors after roughly 125–127 seconds.

A completed 261.7-second run used 64 main model rounds and one presentation round, with about 13 seconds of tool time and no approval wait. Its audit included 51 `run_code` calls. These observations locate substantial latency in model/orchestration round trips; they do not prove every round was unnecessary.

Use the existing programmatic batching and independent-read concurrency where dependencies and permissions permit. Fix lost paging/hash metadata first so the model need not request the same information again. Make process polling event-driven (B10). Keep prompt prefixes stable and measure actual provider cache hits. Avoid indiscriminately parallelizing writes or weakening authorization to improve benchmarks.

### Finish moving blocking work off async inference workers

The vault already supplies `load_blocking`, but some inference planning and subscription paths still call synchronous vault/keyring access. Slow native credential-store calls can occupy async workers. Large workspace scans, parsing and SQLite operations also need measured bounds and appropriate blocking execution rather than a general rewrite of fast file tools.

Measure before changing scheduling: the saved warm request preparation was small. Cold credential access and large-project workloads are distinct cases that were not benchmarked against a real locked keyring in this review.

### Consolidate backend execution policy

The same concerns are implemented independently in foreground chat, prefetch, child dispatch, programmatic calls, hooks, formatting and connector HTTP: timeout, cancellation, process cleanup, output bounding, artifact persistence and side-effect certainty. The reproduced differences show why fixes need shared helpers and contract checks across these entry points.

Keep the existing tool-envelope and dispatch contracts as the shared boundaries. Add execution policy with explicit deadlines, result/lifecycle limits and durable settlement; do not add another competing envelope format. Preserve the distinction between a successful process launch and a completed command.

## Suggested repair order

1. **Protect data and scope:** snapshot parent confinement, hook workspace ownership and process cleanup, exact compaction-plan commit, correct retained-history semantics.
2. **Protect task truth:** stop auto-completing todos; preserve formatter failures; correct fork boundaries and artifact lineage.
3. **Make read/write/tool flows consistent:** align input-size contracts, preserve file identity in bounded results, handle artifact-save failures, drain shell output and make cursors progress.
4. **Reduce waiting and improve recovery:** event-driven process polling, coherent MCP time budgets, effective local slot capacity, subscription refresh settlement, failed-round continuation.
5. **Measure the resulting product:** first model token, first visible answer, model round count, provider time, tool time, approval time, verification outcome and task completion rate. Compare the same tasks before and after each change.

## Validation and limits

The production Rust suite from the earlier investigation passed **1,037 tests, 0 failed, 34 ignored**; Rust check and frontend typecheck passed. This expanded audit additionally ran **19 selected normally ignored loopback integration fixtures: all passed**. They exercise scripted-provider turns, authorization changes, cancellation, live/journal reasoning, incomplete tool calls, actual temporary file saving, child authorization, and native HTTP response handling. They launch no browser and use no paid inference. Other ignored tests are not claimed as passing.

The isolated `audit_review_` checks deliberately assert the observed defective behavior so the reproduction run succeeds when a bug is present. Their passing result is evidence that a defect reproduced, not that it was repaired or that production regression coverage is complete. Use the accompanying exact output for the final fixture count and timing observations. Timing measurements are illustrative local fixtures, not service-level guarantees.

The earlier full frontend run had **891 passed, 1 failed, 1 skipped**. The failure is the live-reasoning integration test in `src/App.test.tsx`, specifically its immediate `Thinking…` assertion; it also failed alone. It remains unresolved and the full frontend suite is not green. This audit does not attribute that failure to the earlier patches without a clean baseline.

No claim is made that this review proves the absence of further defects. The evidence supports concrete repairs to execution, context and persistence; it does not support blaming every model failure on the application or declaring the provider blameless.
