# Harness reliability fixes — 8 November 2026

The twelve finding families in [the original audit](/home/papan/Desktop/Projects/localLM/docs/HARNESS_RELIABILITY_AUDIT_2026-11-08.md) have been addressed in the working tree. The five P1 findings have regression coverage. Final backend and frontend suites pass, including the previously failing verification and permission-change fixtures.

This is a repair report for those findings, not a claim that every possible application defect has been found. The original audit and its defect-reproduction fixtures remain historical evidence.

## Repairs

| Finding | Implemented behavior | Verification |
| --- | --- | --- |
| H01: orphaned child lifecycle | A child lease owns admission, registration and settlement. Initialization errors become failed outcomes. Dropped workers retire their registry entries and persist interrupted lineage with outcome-unknown run state. Startup reconciles both tables. Dispatched calls have durable records; unsettled calls receive an uncertainty notice rather than being replayed. Complete child answers are saved before completion is certified. | Foreground abort during HTTP; asynchronous background preparation failure; invalid snapshot rejection; restart reconciliation; existing interrupted-tool tests. |
| H02: surviving descendants | Linux managed commands join a private group anchored by a guardian. App death closes the ownership pipe and the guardian kills the group. Normal owner drop also kills remaining descendants. Graceful app exit awaits termination acknowledgements separately from output and verification sealing. Profilers signal the actual managed group and retain graceful trace flushing. | Real subprocess owner exit; normal leader exit with a live descendant; acknowledged stop; existing descendant cancellation/registry-drop tests; real Node inspector capture. |
| H03: multimodal request constraints lost during compaction | Handoffs extract text from multipart content and retain archive/message/part references for attachments. The original archived content remains available. Manual compaction uses the stored model content representation. | Multimodal constraint/reference regression plus existing active-exchange, repeated-compaction and durable-rebuild tests. |
| H04: missing child project rules | The common child prompt builder loads the workspace instruction chain and selected skill instructions, subject to capability settings. Foreground and background drivers use it; workflows use those same drivers. | Captured foreground and background requests contain their project-rule sentinels. |
| H05: incomplete work labelled non-partial | Budget/continuation exits propagate partial status. Turn outcomes include unfinished task-graph entries as well as todos and verification status. | Actual 64-request fixture reports partial, preserves its tool schema, and disables tools on the final request; pending task-graph fixture reports its unfinished task. |
| H06: phantom running research after completion | Turns settle their owning research ledger. A new binding table ties ledgers to runs; recovery reconciles older rows and no longer stops after fifty records. Budget-exhausted checkpoints are discoverable for resume. Staging retains a durable pause; the resumed turn reuses the original ledger and transfers its run binding. | Completed answer/restart; fifty-five historical completed ledgers; budget resume; actual paused and budget-exhausted resumed turns each retain one ledger and settle it. Existing interrupted-operation recovery remains covered. |
| H07: empty child output accepted | Both drivers reject an empty visible final answer, including reasoning-only output. The failed child retains any reported usage. | Reasoning-only completed-stream regression, with failed lineage and preserved cache counters. |
| H08: unbounded background children | Foreground and background child leases share an eight-child admission limit per application database in this process, across parents. Admission fails explicitly when full. Permits are released on settlement, failure and drop. A partially admitted parallel workflow interrupts its earlier branches on admission failure. | Eight simultaneous children accepted; ninth rejected; interrupted children settle; replacement admission succeeds. Parent session slots remain independently accounted for. |
| H09: unnecessary verification after read-only PTC | Flat and executable plans report effects derived from typed tool metadata. Trusted reads without configured hooks preserve verification state. Writes, opaque calls, hooks and missing effect metadata remain conservative. Large dispatch futures are heap allocated to avoid the stack overflow exposed during this repair. | Production chat-loop comparison; parallel-plan reads; hook-bearing plans; missing/possible-effect and previous-mutation verification tests. |
| H10: missing delegated inference usage | Each child model request requests and records usage, cache counters and timing. Dropped streams retain incomplete measurements; absent provider counters stay unknown. Response diagnostics display delegated usage separately. Background settlement and parent finalization refresh saved parent metrics, with serialized publication to prevent stale sibling updates. | Three-round child preserves all usage and tool events; incomplete aborted request; background completion before and after parent finalization; two new frontend diagnostics tests. |
| H11: quoted handoff text overwrites context | Only internally attached checkpoint metadata restores handoff state. User or assistant prose quoting the display delimiter is ordinary content. Internal metadata is stripped at the provider request boundary. | Literal-user-handoff regression; repeated handoff/rebuild and payload-boundary tests. |
| H12: vulnerable npm packages | Zod is pinned to 4.6.7 and the lockfile includes the dependency advisory fixes. | Full `npm audit --json`: zero reported vulnerabilities. Frontend tests, typecheck and production build pass with the updated dependencies. |

Concurrent-child testing also exposed SQLite read-to-write upgrade contention and event-number allocation hazards. Completion-message writes, uncertain-child settlement and delegated-metrics publication reserve the writer before reading. Run-event allocation and insertion now use a single SQL statement. An eight-connection regression preserves all 160 messages in timestamp order and all 160 run events without sequence collisions.

Subscription-backed inference now resolves saved credentials on every model request. A long turn can use a rotated token without replaying its tool history. A changed account stops the next request, and both original and current tokens are redacted from stream errors. The regression uses an isolated encrypted fixture vault and loopback HTTP; the existing credential-refresh tests exercise refresh validation and durable rotation. No real account was used.

## Final validation

Evidence is saved in [the validation directory](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08), with [reproduction commands](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/validation-commands.txt).

| Check | Result | Evidence |
| --- | --- | --- |
| `cd src-tauri && cargo test --lib` | **1,091 passed, 0 failed, 35 ignored**, with default test concurrency | [Rust suite](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/rust-suite.txt) |
| Selected normally ignored HTTP/provider/child tests and real Node profiler | **21 passed, 0 failed**; these overlap the 35 ignored above | [Integration run](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/loopback-integration.txt) |
| `cd src-tauri && cargo check --lib` | Passed | [Rust check](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/rust-check.txt) |
| `npx vitest run` | **909 passed, 1 skipped** | [Frontend suite](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/frontend-suite.txt) |
| `npm run typecheck` | Passed | [Typecheck](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/typecheck.txt) |
| `npm run build` | Passed; existing large-chunk warning remains | [Build](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/frontend-build.txt) |
| Node transport/render/storage/search/weather tests | **29 passed** | [Node tests](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/node-tools.txt) |
| Python latency benchmark-runner tests | **10 passed** | [Runner tests](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/benchmark-runner-tests.txt) |
| Full npm audit | **0 vulnerabilities reported** | [Audit JSON](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/npm-audit.json) |
| Whitespace validation | `git diff --check` passed | [Integrity manifest](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/source-integrity.json) |

Twenty-five backend regressions and two frontend tests were added. Regression development initially reproduced a stack overflow, an incompatible guardian kill command, child-fixture ownership omissions and concurrent completion contention. Those were repaired before the final green run. The permission-change fixture now supplies the required third model response while keeping its result-preservation assertions. The historical-observation test waits for process/output finalization before making its after-completion edit. The origin/offline-policy test no longer relies on external DNS.

## Efficiency and long-task conclusions

The controlled two-file fixture now records the following [observations](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/scripted-batching.json):

| Measurement | Direct reads | PTC batch |
| --- | ---: | ---: |
| Model requests | 3 | **2** |
| Outer model-issued tool calls | 2 | **1** |
| Underlying file reads | 2 | 2 |
| Verification reminders | 0 | 0 |
| Final answer | `alpha beta` | `alpha beta` |
| Partial outcome | false | false |

The fixture selects the plan using scripted responses. It proves the orchestration saving and result parity for this case. It does **not** establish better task accuracy, fewer calls on arbitrary tasks, or superiority over another harness. Child cache counters are now visible and persistent; this does **not** establish a higher live cache-hit rate.

The fixes improve cancellation, crash ownership, checkpoint retention, resume identity and credential rotation. They do **not** add an autonomous multi-day execution supervisor or remove the existing model-round, child-round and individual-command limits. No one-hour, two-hour or multi-day soak, real-provider comparative benchmark, or real local-model cache benchmark was run. Treat those capabilities as unverified until measured under representative workloads.

The remaining fourteen ignored Rust tests, the skipped frontend file, Windows/macOS process behavior, installer/native UI acceptance and sustained UI performance were not validated by these runs. No browser preview was launched. Linux process groups cover ordinary descendants; this is not a sandbox against a program deliberately escaping its group.

Rust dependency advisory coverage is still incomplete: `cargo-audit` was unavailable, and [installing it failed](/home/papan/Desktop/Projects/localLM/docs/audits/harness-fixes-2026-11-08/cargo-audit-install.txt) because crates.io could not deliver a dependency before its network deadline. The clean npm audit applies to npm packages only. No claim of a clean Rust advisory scan is made.

Existing uncommitted work was preserved. The changes were compared against the starting source snapshot, and the graphify graph was refreshed using AST extraction without an LLM call. No commit or deployment was performed.
