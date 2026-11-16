# LocalLM inference, tool calling, and response experience audit

Implementation follow-up: application repairs are now recorded in [the fix results](INFERENCE_AND_RESPONSE_FIX_RESULTS_2026-11-08.md). This document retains the original audit/plan as historical evidence.

Reviewed 8 November 2026 against `609209e` **plus the existing working-tree changes**. This is an audit and repair specification; production application files were not changed by this review.

LocalLM has concrete remaining bugs. The most important are lost Chat Completions reasoning during tool replay, accidental interpretation of quoted examples as tool calls, premature success after repetition detection, inconsistent tool input limits, and incorrect live tool identity. The frontend also misclassifies writes, omits some activity and file changes, and presents running processes as finished. These affect both task quality and the user's ability to understand what happened.

**16 open correctness findings** are listed below: 10 backend/build findings and 6 frontend findings. Context retention and 3 experience concerns are tracked separately as design work. Frontend response quality is a first-delivery priority in the [implementation plan](/home/papan/Desktop/Projects/localLM/docs/INFERENCE_AND_RESPONSE_FIX_PLAN_2026-11-08.md).

## Evidence and scope

The [source inventory](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/source-inventory.csv) records 574 source files and 160,503 lines, including tests: 329 frontend files, 136 Rust files, 86 scripts, and 23 end-to-end files. Inventory is codebase-wide; detailed tracing concentrated on the requested inference/tool/response paths. This is not a claim that every line or feature was exhaustively exercised.

The review traced:

| Area | Paths and review focus | Evidence level |
| --- | --- | --- |
| Turn assembly and generation | `chat.rs`, `inference.rs`, `prompt.rs`, provider controls, retries, SSE, finish reasons, recovery | Deep source tracing, library suite, diagnostic stream fixtures |
| Tool calling | Tool assembler, text fallback, discovery, schemas, dispatch, PTC, foreground/child paths | Deep tracing and isolated reproductions |
| Context and persistence | History replay, compaction, artifacts, journal, fork/session state | Deep tracing of model-visible boundaries; existing regression suite |
| Workspace and execution | Read/write aliases, nested instructions, transactions, process lifecycle, hooks, formatter, snapshots | Boundary review, targeted file fixtures, library suite |
| MCP and provider state | Input ownership, transport budgets, connection controls, credentials, local slot capacity | Source review and existing active tests; no real-account acceptance |
| Conversation UI | `App.tsx`, `Chat.tsx`, timeline, reasoning, commands, source links, changed files, replay, status | Deep tracing, component/App diagnostics, 894 active frontend tests |
| Rendering and rich output | Markdown, code/diff/ANSI primitives, tokens/CSS, visual formatter/fallback | Source review, existing tests, historical saved captures |
| Web pipeline and adjacent product areas | `src/web`, model/runtime/settings/tool modules, scripts and E2E configuration | Inventory, integration-boundary review, existing suites; lower depth than core paths |

The 21 additional isolated checks comprise 11 Rust diagnostics, 9 frontend diagnostics and 1 frontend interaction control. A passing **diagnostic** confirms defective behavior; it does not mean the defect has been repaired. Their [instructions and exact results](/home/papan/Desktop/Projects/localLM/docs/audits/2026-11-08/README.md) are retained.

No paid inference, account authentication, production database mutation, or browser preview was performed. Scripted HTTP fixtures use disposable local servers. TCP binding worked on this host; this does not change the restrictions recorded for other sandboxed sessions. Ignored tests remain unverified. The prior refusal of localhost browser preview was respected.

## Open backend and build findings

Priorities: P1 = repair first because generation, execution intent, buildability, or result truth can be wrong; P2 = a specific workflow or presentation contract fails. Severity is conditional on the affected path being used.

### B01 · P1 · The current frontend does not typecheck

**Reproduced by `npm run typecheck`.** [Chat.tsx:42](/home/papan/Desktop/Projects/localLM/src/components/Chat.tsx:42) uses `String.replaceAll`, while [tsconfig.json:3](/home/papan/Desktop/Projects/localLM/tsconfig.json:3) selects ES2020 libraries. TypeScript reports TS2550. This call is in the pre-existing uncommitted changes, which the audit preserved.

The frontend tests pass because they transpile without this typecheck. The normal build includes TypeScript, so this is a release blocker even though component tests are green. Repair the call with an ES2020-compatible replacement, or deliberately raise the supported target and validate the shipped WebViews. The smaller repair is appropriate here.

### B02 · P1 · Chat Completions reasoning disappears from subsequent tool requests

**Reproduced at the stream-to-replay boundary.** Chat receives `reasoning_content`/`reasoning` and displays it, but [sse.rs:208](/home/papan/Desktop/Projects/localLM/src-tauri/src/sse.rs:208) constructs canonical assistant history from only text and calls. Native preservation in [history.rs:22](/home/papan/Desktop/Projects/localLM/src-tauri/src/history.rs:22) covers Responses and Claude content, not compatible Chat Completions reasoning. The next tool round therefore loses reasoning that was actually received.

This is a protocol compatibility bug for endpoints requiring reasoning replay. DeepSeek documents that requests carrying tools must return prior `reasoning_content`, and that omission can cause HTTP 400. Other compatible providers may accept the request but receive less context. This finding does **not** establish that the historical River timeouts were caused by this defect. [DeepSeek thinking/tool protocol](https://api-docs.deepseek.com/guides/thinking_mode/#tool-calls).

Repair with explicit provider-aware reasoning preservation, including persistence and rebuild, rather than treating rendered UI reasoning as an interchangeable native payload. Verify parent and child requests across tool rounds and reopen.

### B03 · P1 · Quoted tool examples become executable calls

**Reproduced with an explanatory fenced XML example.** [tool_calls.rs:319](/home/papan/Desktop/Projects/localLM/src-tauri/src/tool_calls.rs:319) parses `<tool_call>` and `<function=...>` anywhere in text. The masking helper at [tool_calls.rs:9](/home/papan/Desktop/Projects/localLM/src-tauri/src/tool_calls.rs:9) protects only OpenUI fences. [chat.rs:2356](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:2356) invokes this fallback whenever a non-finalizing round has no structured calls, including remote backends.

A response saying “Example syntax, do not execute” followed by a normal XML code fence becomes `TextCallDecision::Execute`; the example is also removed from the answer. Ordinary approval and dispatch restrictions still apply: this is an intent/control parsing defect, **not a demonstrated permission bypass**.

Restrict compatibility fallback to an explicitly selected protocol and distinguish actual control output from code, quotations, inline examples and malformed text. Use the same classification for parsing, display suppression, truncation recovery and children.

### B04 · P1 · Repetition detection can truncate valid code and report success

**Reproduced with two standalone HTML templates sharing required boilerplate.** [chat.rs:519](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:519) finds repeated normalized suffixes without considering code fences or requested duplication. The probe's valid 2,339-byte response was cut to 1,225 bytes inside the second example.

The caller breaks the stream at [chat.rs:2153](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:2153), then assigns `finish_reason = "stop"` at [chat.rs:2182](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:2182) when no structured calls are present. That can enter normal completed-answer handling despite ending before the provider's terminal result.

Retain a bounded loop guard, but protect code/data and intentional repetition. A heuristic interruption must produce an explicit partial/interrupted outcome and a bounded recovery decision. It must not fabricate a successful provider stop.

### B05 · P1 · Tool discovery imposes a smaller, undocumented write limit

**Reproduced with a valid 40,000-byte content value.** Direct workspace write arguments pass the current per-tool checks, but [tool_discovery.rs:126](/home/papan/Desktop/Projects/localLM/src-tauri/src/tool_discovery.rs:126) rejects the discovery envelope above 32 KiB. The stream assembler also assigns the generic 65,536-byte allowance to `tool_invoke`, before the target is resolved. See [tool_spec.rs:19](/home/papan/Desktop/Projects/localLM/src-tauri/src/tool_spec.rs:19).

Large catalogs require deferred tools to be invoked through this wrapper. `workspace_write_file` is not in the everyday direct-advertisement list, making this reachable when optional tools expand the catalog. The prior direct-write limit fix therefore does not solve every model-facing route.

Define one target-aware argument contract through streaming, discovery, PTC and dispatch. Include envelope/escaping overhead in bounded wrapper transport; revalidate against the resolved authorized tool. Keep per-call and per-round memory bounds.

### B06 · P1 · Local execution can leave stale checks marked verified

**Reproduced in the real verification bookkeeping.** [verification_progress.rs:52](/home/papan/Desktop/Projects/localLM/src-tauri/src/verification_progress.rs:52) recognizes selected write/opaque aliases, but misses `local_run_code` and `local_execute_command`. Those are actual aliases from [connectors.rs:263](/home/papan/Desktop/Projects/localLM/src-tauri/src/connectors.rs:263).

After a workspace edit and a passing check, observing either local execution alias with development-task context leaves the outcome `verified` and `needs_verification = false`. Such code or commands can change files after the check. The fixture exercises bookkeeping, not an actual external mutation.

Use tool execution metadata and observed workspace revisions instead of another independent name list. Invalidate earlier evidence after an applicable opaque operation; preserve the distinction between a successful command, a recognized passing suite, and source-bound verification. Final check freshness also needs to account for post-tool hooks that may mutate files after bookkeeping.

### B07 · P2 · Child results mix intermediate content with the final answer

**Reproduced through scripted HTTP streams in two checks.** [subagents.rs:2355](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:2355) appends every content chunk into one cumulative answer. The driver later validates/returns that string at [subagents.rs:1394](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:1394).

First, a tool-round commentary sentence followed by valid final JSON becomes `I will inspect the file.{"ok":true}` and fails `outputSchema`, even though the final reply matches the schema. Second, inline `<think>...</think>` from a local-style model stays in the returned answer because this collector lacks the parent's thinking filter. Explicit API reasoning is also subject to B02's replay problem.

Keep commentary, exposed reasoning, calls and final output as separate round items. Validate the actual final answer. Share protocol interpretation with the parent while retaining different presentation/authority policies for child work.

### B08 · P2 · A child's text fallback breaks native Responses history

**Reproduced through a scripted native Responses stream.** [subagents.rs:2373](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:2373) accepts text-parsed calls, then [subagents.rs:2383](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:2383) retains the original native output containing only text. The canonical history now has a function call absent from that native output.

The next request correctly fails the parity check in [responses_adapter.rs:111](/home/papan/Desktop/Projects/localLM/src-tauri/src/responses_adapter.rs:111): `Saved Responses function calls conflict with canonical history.` The parent already removes incompatible native metadata for text fallback at [chat.rs:2705](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:2705).

Apply the same canonicalization policy to children. If B03 disables fallback for native protocols, retain a compatibility regression for explicitly enabled fallback so malformed native/canonical mixtures cannot return.

### B09 · P2 · Nested project rules depend on which read tool is used

**Reproduced with a temporary nested `AGENTS.md`.** Harness `file_read` appends applicable nested rules at [harness.rs:495](/home/papan/Desktop/Projects/localLM/src-tauri/src/harness.rs:495). Native `workspace_read_file` instead goes directly through [connectors.rs:649](/home/papan/Desktop/Projects/localLM/src-tauri/src/connectors.rs:649), returning the same file without those instructions. Turn assembly loads the root/ancestor chain, so it does not fill this descendant-rule gap.

The fixture confirms that the nested rule appears with `file_read` and disappears with `workspace_read_file`. This makes instruction adherence depend on tool selection.

Attach applicable file instructions through one trusted workspace boundary for all supported read/edit paths, with identity checks, deduplication and bounded output. Ensure instruction metadata survives result bounding; discovered tools and PTC must have the same behavior.

### B10 · P2 · Bounded harness file reads still lose the file hash

**Reproduced with a real temporary file and a 2 KiB output budget.** The newer preservation helper at [artifacts.rs:780](/home/papan/Desktop/Projects/localLM/src-tauri/src/artifacts.rs:780) accepts `read_file`/`workspace_read_file` in the Workspace family. First-party harness `file_read` is normalized as Harness and misses this path.

Its complete result has the file's SHA-256, but the bounded result has neither top-level nor canonical `data.sha256`; it instead supplies an artifact pointer. The file hash remains recoverable from the full artifact, so this is avoidable context loss and rereading rather than file corruption. Artifact checksums must not be mistaken for file checksums.

Extend trusted file-identity preservation to this native alias without granting arbitrary MCP results native semantics. This is the remaining harness-alias case of the older B16 finding.

## Open frontend correctness findings

### F01 · P1 · Reused call IDs merge distinct live commands and output

**Reproduced against the production `App` with separate round IDs.** [App.tsx:868](/home/papan/Desktop/Projects/localLM/src/App.tsx:868) keys transient messages by `tool-${toolCallId}`. `started` skips an existing ID, and [toolOutput.ts:12](/home/papan/Desktop/Projects/localLM/src/lib/toolOutput.ts:12) buffers by conversation/call only. Neither includes run or step identity.

The fixture sends `textcall_0` in `step-0` and `step-1`. Only one row remains, its header still names the first command, and its terminal combines both commands' output. This ID is realistic: [tool_calls.rs:397](/home/papan/Desktop/Projects/localLM/src-tauri/src/tool_calls.rs:397) restarts generated text-call IDs for each parsed batch. Reloaded durable data can repair the display, but it is wrong during the run.

Use a durable operation identity including conversation, run, round/step and call. Carry it through events, transient buffers, result settlement, reducer keys and replay matching.

### F02 · P2 · A partial journal hides otherwise valid saved tool messages

**Reproduced in the production selector.** Once any `tool_call` event exists, [ActivityTimeline.tsx:91](/home/papan/Desktop/Projects/localLM/src/components/ActivityTimeline.tsx:91) returns only `calls.map(...)`. Saved tool messages without matching call events disappear. The fallback for message-only tools operates only when the entire event call list is empty.

This affects legacy/partial journals and other message-only settlements. A temporary journal write problem should not hide an otherwise persisted write or result. Merge unmatched records into a normalized timeline once, preserving chronology and avoiding duplicate rows. Counts, files, sources and detail views should consume the same normalized operations.

### F03 · P2 · Native writes and web research receive incorrect activity categories

**Reproduced by classification and actual timeline rendering.** [TraceIcon.tsx:62](/home/papan/Desktop/Projects/localLM/src/components/TraceIcon.tsx:62) recognizes `write_file` but not native `file_write`; a path then makes it a read. Two successful writes are displayed inside a collapsed **“Read 2 files”** group. AREX `search` becomes file-search activity and `visit` becomes a generic tool. [CommandRunCard.tsx:228](/home/papan/Desktop/Projects/localLM/src/components/CommandRunCard.tsx:228) independently repeats similar heuristics.

These labels do not change backend permission classification, but they misrepresent consequential actions to the user. Introduce authoritative tool-family/category metadata with explicit alias mappings and one frontend renderer registry. Verify every first-party alias; retain a safe generic MCP fallback.

### F04 · P2 · Successful native changes are absent from the file review

**Reproduced with current native result shapes; earlier UX finding remains open.** [WorkSummary.tsx:23](/home/papan/Desktop/Projects/localLM/src/components/WorkSummary.tsx:23) requires `request.arguments.path` and one `data.diff`. Workspace transactions return `data.changes[]` with per-file paths/diffs at [workspace_patch.rs:1609](/home/papan/Desktop/Projects/localLM/src-tauri/src/workspace_patch.rs:1609). Harness `file_write` also does not match its tool-name expression.

The fixture's successful batch patch and file creation count as actions but produce no file-change section. This is not limited to multi-file patches: newer transaction-backed write/append/edit variants use the changes array too.

Normalize single/batch create, edit, delete and move results into a shared changed-file model. Preserve before/after identity and distinguish accumulated edit counts from final net diff. Represent shell-created files only when observed; do not infer them from prose.

### F05 · P2 · Recovered sources do not repair inline citations

**Reproduced with an artifact-backed web result.** [AnswerSources.tsx:51](/home/papan/Desktop/Projects/localLM/src/components/AnswerSources.tsx:51) fetches missing full metadata into its own local state. Chat computes the inline source map separately from bounded messages/events at [Chat.tsx:577](/home/papan/Desktop/Projects/localLM/src/components/Chat.tsx:577).

After artifact hydration, the source footer has an actionable source, while the answer's `[s1]` remains plain unresolved text. Research context consumers also use the pre-hydration set. This case is reachable when bounding spills source metadata; it is not an assertion that every search loses its sources.

Use a conversation/turn-owned evidence store or hook shared by prose citations, source footer, research context and tool detail. Preserve original citation IDs and expose ambiguous or unavailable references without inventing attribution.

### F06 · P2 · A running process is presented as finished

**Reproduced through the production Chat renderer.** A completed `process_start` request can return `data.state = "running"`, `exitCode = null`, and output frames, as defined at [process_tools.rs:181](/home/papan/Desktop/Projects/localLM/src-tauri/src/process_tools.rs:181). [Chat.tsx:135](/home/papan/Desktop/Projects/localLM/src/components/Chat.tsx:135) uses the tool-message delivery status to choose `completed`, and the card displays **Finished**.

The same distinction matters for `shell_run` returning a live process handle after its wait budget. Tool request completion, process completion and check outcome are different states. Process frame arrays also fall back to JSON instead of the existing terminal channel renderer.

Add a process-aware view with explicit Started/Running/Exited/Stopped/Unknown states, cursor-backed output, and owned stop behavior. Reconcile later polling results into that same process view rather than adding a new apparent command for every poll.

## Generation quality gap and frontend design concerns

### Q01 · High priority · Compaction archives history but does not preserve a useful semantic handoff

**Reproduced as an information-retention limitation, not archive corruption.** Live compaction retains only short excerpts of the last eight dropped messages at [compaction.rs:223](/home/papan/Desktop/Projects/localLM/src-tauri/src/compaction.rs:223). Rebuilt history can use a generic archive notice at [history.rs:120](/home/papan/Desktop/Projects/localLM/src-tauri/src/history.rs:120). An old explicit requirement in the fixture survives in the archive but is absent from the next model-visible checkpoint.

`TaskCheckpoint` and `ResearchStateSummary` helpers exist, but their current construction callers are tests; they are not the production automatic-compaction handoff. Archive access is a valuable recovery mechanism, but it does not replace a concise working summary of requirements, decisions, paths, completed operations, unresolved work and sources. The active tool exchange is also retained as a whole; very long single-turn work needs a separately designed compaction strategy.

Implement semantic handoff with durable provenance and safe fallback. Verify actual remembered requirements on later turns; helper round-trip tests alone do not establish generation quality.

### U01 · First delivery · Collapse behavior and status hierarchy are inconsistent

Two UI diagnostics establish the mechanics: a command opened automatically while running remains expanded after success; mounting its completed history defaults to collapsed. Separately, [ActivityTimeline.tsx:153](/home/papan/Desktop/Projects/localLM/src/components/ActivityTimeline.tsx:153) still treats any `partial` envelope as attention-worthy, even though Chat no longer mislabels clipped success as failure. This prevents successful bounded reads from grouping normally.

Define an explicit default-open policy that preserves deliberate user inspection. Keep the current action, approval and error visible; summarize settled exploration; make raw JSON a secondary inspection view. Use one understandable active status instead of simultaneous generic and tool-specific indicators.

### U02 · First delivery · The response needs a stronger reading hierarchy

The saved 5–6 November captures show the previous implemented surfaces, not the exact current dirty tree. The light 1280×840 fixture has limited transcript height after header, project line, setup hint, composer and footer. Current CSS already improves Markdown, code, category tokens and activity rows. Those improvements should be retained.

The remaining design target is a quiet chronological work trail, clearly subordinate reasoning, compact command summaries with readable output on inspection, useful web query/source summaries, accurate changed-file review, and a strong final answer. Approvals should show the action and scope before raw arguments. Typography, spacing and error states need native visual acceptance in both themes; this audit did not establish current pixel-level quality.

### U03 · Next delivery · Hidden-session attention and long-history responsiveness need acceptance work

[App.tsx:208](/home/papan/Desktop/Projects/localLM/src/App.tsx:208) selects an approval across sessions and [App.tsx:1250](/home/papan/Desktop/Projects/localLM/src/App.tsx:1250) mounts one modal for it. Background approvals can therefore interrupt reading in another chat. Route attention by owning conversation while preserving backend authorization.

Every live journal merge still revisits the timeline, and the transcript mounts all turns. Closed reasoning also mounts its Markdown subtree. Existing tests cover useful parse reuse and terminal bounds, but do not measure sustained interaction latency in a long native session or hidden-session update pressure. Profile those cases before choosing virtualization or a state-store rewrite. These are source-supported risks and missing acceptance measurements, not measured performance failures.

## What to take from OpenCode and Claude Code

OpenCode was inspected from the official repository at commit **`ece56ca58419083275f2a2a81ac28d05f4a1b89b`**. Its current source has explicit typed timeline rows and stable part identities, separate context/read/file groups, tool-specific renderers, controlled disclosure state, and cursor-based shell following. The useful lesson is to normalize operations once and give each kind of work an appropriate compact representation. This is source evidence, not a live visual comparison. [Timeline identity](https://github.com/anomalyco/opencode/blob/ece56ca58419083275f2a2a81ac28d05f4a1b89b/packages/session-ui/src/timeline/timeline-row.ts), [tool grouping and renderers](https://github.com/anomalyco/opencode/blob/ece56ca58419083275f2a2a81ac28d05f4a1b89b/packages/session-ui/src/tools/tool-renderer.tsx), [shell following](https://github.com/anomalyco/opencode/blob/ece56ca58419083275f2a2a81ac28d05f4a1b89b/packages/session-ui/src/tools/shell-output.ts).

Claude Code's official desktop documentation describes a normal transcript with grouped tool summaries and complete answer text, deeper thinking/verbose inspection, and a dedicated changed-file diff view. Adopt the hierarchy and progressive inspection, with visible failures and required actions. Do not assume its model quality follows from its UI, and do not copy a settings menu merely because the reference has one. [Claude Code desktop documentation](https://code.claude.com/docs/en/desktop#switch-view-modes).

**Local product constraint remains binding:** response presentation is configured in Settings. No Auto/Text/Visual selector, equivalent response-mode control, or per-conversation presentation editing is proposed for the homepage/composer. The accompanying synthetic response proposal is a design reference, not an implementation or a screenshot of either product.

## Reconciliation with earlier audits

The older backend inventory has 105 identical source files and 31 changed files relative to this review. Historical findings cannot be treated as the current open backlog.

| Earlier 6 November finding | Current disposition |
| --- | --- |
| B01/B02 archive commit and retention bugs | Accepted-plan commit and retained-window handling are now present; preserve their regressions. Q01 is a separate semantic-retention gap. |
| B03 snapshot parent traversal | Restore uses scoped handling and has a parent-link write/delete regression. Do not reopen based only on the old report. |
| B04/B05 hook workspace/process deadlines | Current hooks receive the execution workspace and use managed capture/deadlines. |
| B06/B14/B15 branch boundaries, artifact ownership and event paging | Current store code reconstructs boundaries, rebinds reachable artifacts and pages events. |
| B07 direct write argument limit | Direct workspace budgets were expanded. Current B05 records the remaining discovery-wrapper mismatch. |
| B08 todo auto-completion | Removed; current turn settlement retains unfinished todos and explicit outcomes. |
| B09/B10/B11 shell draining, polling and frame progress | Current code drains pages, uses notifications and has decoded-frame progress coverage. |
| B12/B13 formatter deadline/status | Managed capture and explicit failure reporting are present. |
| B16 file identity bounding | Workspace case improved; current B10 reproduces the remaining harness `file_read` case. |
| B17 artifact save failures | Current paths use `capture_tool_result` and explicit capture-failure metadata. |
| B18 MCP human-input budget | Execution/input budget separation is now present; real long-running authenticated interactive acceptance remains outstanding. |
| B19 local slot capacity | Runtime queries effective per-slot context and rejects unusable properties. No live GPU acceptance was performed here. |
| B20 subscription refresh | Serialized, validated, durable refresh handling and a transport fixture now exist. No real account was exercised. |
| Earlier frontend immediate `Thinking…` test failure | Current frontend suite passes. Do not reintroduce an obsolete label solely to satisfy historical evidence. |

These dispositions reflect source changes and current active regression coverage, not a new proof of every external integration.

## Validation results and practical limits

| Check | Observed result |
| --- | --- |
| `npx vitest run` | 894 passed, 1 skipped; 119 passing files, 1 skipped file |
| `npm run typecheck` | Failed, TS2550 in `Chat.tsx:42` — B01 |
| `cargo check --lib` | Passed |
| First `cargo test --lib` | 1,049 passed, 1 failed, 35 ignored; cancellation timing assertion failed |
| Isolated cancellation test | Passed in 0.60 s |
| Full Rust rerun with `--test-threads=2` | 1,050 passed, 0 failed, 35 ignored |
| Added Rust diagnostics, disposable source copy | 11 passed; defective behavior reproduced |
| Added frontend diagnostics/control | 10 passed; 9 diagnostic checks and 1 interaction control |
| Synthetic response proposal | DOM/state interactions checked without a browser; visual/native acceptance remains pending |

The initial Rust failure is retained in the evidence. Its wall-clock assertion includes setup/turn preparation and passed alone and with reduced suite concurrency. This suggests timing sensitivity; it does not establish a production cancellation defect or justify silently ignoring the failure. Replace timing-only synchronization with an explicit dispatched/interrupt observation and keep a reasonable liveness bound.

Passing suites do not measure comparative answer quality. No same-model LocalLM/OpenCode task benchmark was run. Earlier upstream timeouts remain separate evidence; the remaining application bugs can worsen reliability and context, but cannot by themselves explain every unsatisfactory generation. The implementation plan includes a controlled comparison and native visual acceptance before claiming parity.
