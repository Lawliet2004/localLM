# LocalLM harness reliability audit — 8 November 2026

> Historical pre-fix audit. The subsequent repairs and final validation are recorded in [the fix results](/home/papan/Desktop/Projects/localLM/docs/HARNESS_RELIABILITY_FIX_RESULTS_2026-11-08.md).

**Serious issues remain. LocalLM has useful agent infrastructure, but the current evidence does not support claims of superior task accuracy, consistently fewer model requests, higher inference-cache reuse, or reliable autonomous execution over days.** Hour-long or two-hour work is possible within the present limits while the app stays available; it has not been established by a sustained acceptance test.

This review found **12 actionable finding families**, including five P1 priorities. Eleven concern application behavior or measurement; the twelfth covers dependency advisories and has a different evidence standard. Findings are conditional on the affected feature being used. P1 means repair first because execution ownership, project constraints, or resource control can fail. P2 means a specific correctness, efficiency, reporting, or maintenance problem.

The reviewed state is commit `609209e7c5c3b69a456dca577a7ee6bcc046a0e7` **plus the pre-existing uncommitted changes**. This audit added documentation and disposable test fixtures. It did not repair production code. Hash comparison confirmed that all 572 inventoried original source/test/script files remained unchanged. [Inventory and integrity evidence](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/source-integrity.json).

**Answers to the requested questions**

| Question | Evidence-based answer |
| --- | --- |
| Are there serious bugs? | Yes: lost request constraints during compaction, missing child project instructions, orphaned child lifecycle state, surviving subprocess descendants after owner exit, and unrestricted background-child admission are the first priorities. |
| Does the harness complete tasks with fewer calls? | It supports batching, discovery, independent-read concurrency and artifact recovery. This does not establish fewer calls on real tasks. A controlled two-file fixture reduced outer calls from 2 to 1, but still made 3 model requests in both paths because read-only PTC caused an extra verification round. |
| Does it improve accuracy over other harnesses? | Unestablished. No current same-model, same-task comparative task-success result was produced or found in the reviewed benchmark paths. Unit tests and scripted providers establish orchestration behavior, not model accuracy. |
| Does it have higher cache hits? | It has sound cache-oriented mechanisms, but no comparative live inference-cache result was established. Child usage is currently omitted, preventing complete accounting on delegated tasks. |
| Can it handle one or two hours? | Conditionally: a turn has 64 model rounds, individual operations have much shorter deadlines, and recovery has important limitations. Elapsed runtime alone is not a completion guarantee. No one-hour or two-hour soak was run. |
| Can it reliably work for days unattended? | The present implementation and tests do not establish this. There is no demonstrated durable supervisor that resumes execution across app exit, machine restart, provider outages and round-budget boundaries. Storing a task graph or objective is not such a supervisor. |

**What was actually tested**

The inventory covers 137 Rust files, 330 frontend files, 87 scripts and 18 E2E source files: approximately 164,141 lines including tests. Deep tracing concentrated on generation, tool dispatch, child orchestration, context retention, persistence, process ownership, verification and cache measurement. The rest of the application received lower-depth boundary review and existing-suite coverage. Inventory coverage is not exhaustive behavioral coverage.

| Check | Observed result | Evidence |
| --- | --- | --- |
| `cargo test --lib` | 1,065 passed, **1 failed**, 35 ignored | [Initial Rust run](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/rust-suite.txt) |
| The failing verification test alone | Passed | [Isolated rerun](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/rust-failure-isolated.txt) |
| `cargo test --lib -- --test-threads=2` | **1,066 passed**, 35 ignored | [Reduced-concurrency run](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/rust-suite-threads2.txt) |
| `cargo check --lib` | Passed | [Rust check](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/rust-check.txt) |
| `npx vitest run` | **907 passed**, 1 skipped | [Frontend suite](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/frontend-suite.txt) |
| TypeScript and production frontend build | Passed; build retains large-chunk warning | [Typecheck](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/typecheck.txt), [build](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/frontend-build.txt) |
| Twenty normally ignored local HTTP/provider fixtures | **19 passed, 1 failed**, both concurrent and serial runs | [Initial integration run](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/loopback-integration.txt), [serial rerun](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/loopback-integration-serial.txt) |
| New fault, limit and ownership probes | **15 passed**: 14 diagnostics and 1 acknowledged-stop control | [Probe results](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/probes.txt), [structured observations](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/probe-facts.json) |
| Disposable three-response integration-fixture control | Passed with original assertions retained | [Control](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/loopback-fixture-control.txt), [fixture-only change](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/integration-fixture-adjustment.diff) |
| Node transport/render-mediation/storage/search/weather tests | **29 passed** | [Node checks](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/node-tools.txt) |
| Response-latency benchmark runner tests | **10 passed** | [Runner checks](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/benchmark-runner-tests.txt) |
| Production dependency audit | **1 high-severity package flagged** | [Production audit](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/npm-audit-production.json) |
| Full dependency audit | **3 high and 2 moderate package entries flagged** | [Full audit](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/npm-audit-all.json) |

**A passing diagnostic means the undesirable behavior reproduced. It is not a passing correctness requirement or a repair.** The final probes assert the observed behavior explicitly. Initial probe-development logs contain corrected fixture assumptions and are not counted as additional application failures.

The first Rust failure was `development_tools::tests::observations_finish_without_report_polling_and_remain_historical`. Its output combined completed observation evidence with a process snapshot still marked running, yielding `verifiedCurrent=false`. It passed alone and in the reduced-concurrency full run. This is a timing-sensitive verification/test issue; the initial failure remains part of the record.

The persistent integration failure is `chat::output_recovery_tests::permission_change_after_group_dispatch_preserves_both_real_results`, at the final `outcome.unwrap()`. The unchanged fixture supplies two model responses. Post-tool hooks now invalidate verification, and the completion reminder requests another model round. Supplying a third response in the disposable fixture makes its original result-preservation assertions pass. This identifies an outdated fixture/flow expectation; it does not justify reporting the unchanged suite as green or removing the verification protection.

Loopback sockets worked on this host. These results do not supersede the restrictions documented for other sandboxed hosts. No browser preview or browser route was used. No paid model request, real-account authentication, dependency installation, or production-data mutation was performed. The app database was opened read-only solely to check non-secret local asset settings. No configured local model/runtime asset pair was established, so no live model benchmark was run. The remaining ignored tests, real GPU inference, installer acceptance, Windows/macOS runtime behavior, and sustained native UI performance remain unverified. A Rust dependency advisory scan was not run; `cargo-audit` was unavailable.

**Prioritized findings**

| ID | Priority | Finding | Evidence |
| --- | --- | --- | --- |
| H01 | P1 | Child work can lose terminal lifecycle settlement | Aborted foreground worker, failed background setup, and restart probes |
| H02 | P1 | A subprocess descendant can survive owner process exit | Real managed-process subprocess probe on Linux |
| H03 | P1 | Compaction loses text requirements from multimodal requests | Real compaction planner and checkpoint probe |
| H04 | P1 | Child inference starts without workspace instruction files | Captured child request; parent control retains the rule |
| H05 | P2 | Partial-work reporting misses budget exits and task-graph work | Actual 64-round run and pending-task turn |
| H06 | P2 | Completed turns leave running research ledgers that reopen as paused | Completed-answer/restart probe |
| H07 | P2 | An empty child answer is accepted as completed | Reasoning-only scripted child stream |
| H08 | P1 | Background children bypass parent concurrency admission | Twelve simultaneous accepted children |
| H09 | P2 | Read-only PTC incurs unnecessary verification work | Equivalent two-file paths through the real chat loop |
| H10 | P2 | Child model tokens/cache usage are omitted | Supplied usage counters disappear from child accounting |
| H11 | P2 | Literal user content can replace a compaction handoff | User-message delimiter/JSON probe |
| H12 | P2 maintenance; upstream high advisories | Installed dependencies have published security advisories | npm audit, installed-version check, bounded Zod reproduction |

**H01 — Child lifecycle settlement is not guaranteed**

There are three related failures. Aborting a foreground child's future while it waits on HTTP leaves its `subagent_runs` row `running` and its registry entry accepting mail, even though its worker has ended. A fault-injected background setup error is discarded by the supervisor wrapper: the registry retires, but the saved row remains `running` without an error. After restart, startup recovery cancels the child's `runs` row while leaving its lineage row `running` and no live registry owner.

This can strand delegated work and make status/polling unreliable over long tasks. The setup case deliberately uses an invalid snapshot to exercise failure handling; it is not a claim that normal provider settings accept invalid URLs. No live provider or write replay was involved.

Sources: [foreground child lifecycle](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:1743), [background supervisor](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:1863), [startup recovery](/home/papan/Desktop/Projects/localLM/src-tauri/src/store.rs:731).

Repair: give every child an owner/settlement guard, persist a terminal or outcome-unknown state on dropped work, handle setup errors explicitly, and reconcile both run tables together at startup. Preserve uncertainty for already dispatched side effects; do not automatically replay them.

**H02 — The immediate child death guard does not contain every descendant**

The Linux fixture starts a managed shell that launches a separate Python descendant, confirms the descendant started, calls `stop_all()`, then immediately exits the owner process. The descendant still writes a marker afterward. A separate control that awaits the acknowledged `process_stop` prevents the later write.

`PR_SET_PDEATHSIG` protects the direct child; that property does not automatically give every descendant equivalent containment. `stop_all()` clears control senders without waiting for supervisors to acknowledge process-tree termination. This is a proven crash/immediate-exit gap. The exact timing of ordinary Tauri graceful shutdown was not exercised, and this finding is not asserted for Windows job-object behavior.

Sources: [managed process creation](/home/papan/Desktop/Projects/localLM/src-tauri/src/execution.rs:344), [Linux parent-death guard](/home/papan/Desktop/Projects/localLM/src-tauri/src/execution.rs:432), [unacknowledged bulk stop](/home/papan/Desktop/Projects/localLM/src-tauri/src/process_tools.rs:241).

Repair: await acknowledged tree shutdown during graceful exit and add platform-appropriate crash containment for descendants. A process group plus an in-process destructor is insufficient after that process has already exited.

**H03 — Attached requests lose their text constraints during compaction**

The handoff builder reads only string-valued `content`. The model transcript represents a request containing an attachment as content parts, including a text part. In the probe, the request says `MUST preserve protected.txt` and includes an image. Once that old turn is compacted, the handoff has an empty original request and no constraints; the live kept context no longer contains the requirement.

The verbatim archive still contains the original request. The failure is model-visible retention: nothing in the new handoff identifies that omitted requirement. This matters when long tasks compact repeatedly.

Source: [checkpoint extraction](/home/papan/Desktop/Projects/localLM/src-tauri/src/compaction.rs:85).

Repair: extract text from all supported content representations, retain attachment/artifact identity separately, and test both ordinary-turn and active-exchange compaction plus reopen. Bound memory without silently discarding the task's stated constraints.

**H04 — Project rules do not enter the child's initial prompt**

With a root `AGENTS.md` containing a sentinel rule, the captured parent request includes it. The corresponding child request contains only the saved system prompt, child guidance and capability metadata; it omits the project rule. `child_system_prompt` does not load the instruction chain.

A later workspace read may supply applicable file instructions, but a child can choose execution or answer directly before that happens. The permission engine still applies: this is an instruction-propagation failure, not a demonstrated permission bypass.

Source: [child prompt construction](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:899).

Repair: bind the child's system context to the same canonical workspace instruction snapshot as its parent, with scoped inheritance and explicit refresh rules. Verify foreground, background and workflow children.

**H05 — Unfinished work can be recorded with `partial=false`**

The 64-round probe performs 63 distinct reads, then reaches a final round with `tool_choice=none`. The loop supplies an instruction that a budget/checklist limit has been reached, yet `turn_end.partial` is false. The continuation branch sets a fallback before the later branch that sets `partial_work`, so that flag is skipped.

Separately, a turn creates a pending task through `task_create` and ends without implementing it. The task remains pending, while `turn_end.partial=false` and `unfinishedTodos=[]`. Final bookkeeping checks the older todo list, not the dependency-aware task graph. The UI consumes this partial-work field.

Sources: [partial continuation](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:3365), [budget flag](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:1985), [turn outcome](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:4844).

Repair: propagate an explicit structured stop/completion reason through every exit, and include unresolved owned graph tasks in outcome reporting. Keep successful answer delivery separate from task completion.

**H06 — Completed work reappears as paused research**

A normal completed `2 + 2` answer leaves its research ledger `running`, with `plan/search/read/verify/answer` still pending. Running the real startup recovery converts that ledger to `paused`. The main chat loop creates this ledger at the start but does not settle it on normal completion.

This pollutes recovery state with work that has already finished, undermining progress and resume decisions. Ledger operation names also do not consistently correspond to its generic pending-work names.

Sources: [ledger creation](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:1937), [research recovery](/home/papan/Desktop/Projects/localLM/src-tauri/src/research_tasks.rs:379).

Repair: settle the ledger with the parent run's actual result and retain meaningful pending work only. Reconcile old rows against their owning terminal runs instead of assuming every `running` ledger was interrupted work.

**H07 — Empty delegated output is reported as completed**

A child stream with reasoning, a valid `stop` finish and completion marker, but no visible answer, returns an empty string successfully and persists `status=completed`. The default child contract has no required output schema, so schema validation does not reject it. The parent chat already treats empty final answers as errors.

Sources: [child completion branch](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:1261), [child stream collector](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:2317).

Repair: require a meaningful final result or an explicit structured no-result outcome. Apply the same rule to both child drivers, with bounded recovery and accurate failure status.

**H08 — Background child admission is unrestricted by parent slots**

Twelve background children were accepted simultaneously while consuming zero entries in the four-slot foreground session registry. They were all explicitly interrupted and settled after the probe. The per-child round/depth bounds do not bound concurrent breadth.

Repeated background delegation can therefore exceed the concurrency a caller may infer from the parent registry and overload local inference, connections or a remote provider. This review did not issue paid inference or measure actual provider cost.

Sources: [four parent slots](/home/papan/Desktop/Projects/localLM/src-tauri/src/session_runs.rs:100), [background spawning](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:1863).

Repair: add shared admission and cost/token budgets across parents and children, with fairness, bounded queues, cancellation ownership and terminal settlement when admission fails.

**H09 — Read-only batching loses its expected round-trip saving**

Both fixture paths read `a.txt` and `b.txt`, receive `alpha` and `beta`, and return the same final text through production orchestration. Workspace and Execution sources are enabled in both paths.

| Measurement | Two direct reads | One PTC batch containing the same reads |
| --- | ---: | ---: |
| Outer model-issued tool calls | 2 | 1 |
| Underlying file reads | 2 | 2 |
| Durable tool audit rows | 2 | 3, including the wrapper |
| Model requests | 3 | 3 |
| Verification reminders | 0 | 1 |
| Recorded outcome | Verification not required | Changes unverified; partial work |

`Progress::observe` unconditionally treats `ptc_run` as opaque mutation. Even a batch composed only of trusted reads increments the revision and causes a corrective generation. A model that follows that reminder with an actual check can incur still more work. This deterministic result does not measure how often a real model chooses PTC.

Source: [verification classification](/home/papan/Desktop/Projects/localLM/src-tauri/src/verification_progress.rs:61).

Repair: propagate observed mutation/uncertainty from nested dispatch, retaining conservative handling for writes and unknown operations. Evaluate fewer model requests per successfully completed task, not just fewer wrapper calls.

**H10 — Delegated token and cache usage disappear**

The child fixture supplies 100 input tokens and 80 cached tokens. The child driver neither requests standard streaming usage in its compatible payload nor records an `inference_usage` event from the supplied counters. The collector returns finish state, calls and assistant content, without usage. Main response diagnostics therefore cannot describe total delegated model usage or cache reuse.

This is an observability gap, not evidence of an actual cache miss. It prevents credible end-to-end efficiency comparisons whenever delegation is used.

Sources: [child payload](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:1218), [collector result](/home/papan/Desktop/Projects/localLM/src-tauri/src/subagents.rs:2311).

Repair: retain per-child/per-round reported usage and aggregate it separately with explicit coverage for missing counters. Include retries and unsuccessful attempts in task cost and latency accounting.

**H11 — A literal handoff example can replace retained context**

`task_checkpoint` interprets a matching handoff delimiter followed by valid checkpoint JSON regardless of the message's role or provenance. A user asking to explain such a literal example can replace the earlier original request and erase its retained constraints. The fixture changes `Never modify protected.txt` into an unrelated example with no constraints.

Source: [unqualified delimiter interpretation](/home/papan/Desktop/Projects/localLM/src-tauri/src/compaction.rs:97).

Repair: carry authenticated internal checkpoint metadata outside ordinary message text, or at least require an internally created record with verified provenance. Quoted user/assistant content must remain data. No permission bypass was demonstrated.

**H12 — Dependency security maintenance is overdue**

The installed dependency tree and registry audit report the following package entries. These are package-level advisories, not five demonstrated exploitable application endpoints.

| Installed package | Scope | Audit severity | Required follow-up |
| --- | --- | --- | --- |
| Zod 4.6.5 | Production, direct and shared with OpenUI | High | Patched advisory floor is 4.6.6; audit offers 4.6.7. Update and verify the rendering/schema contract. |
| Vite 8.2.2 | Development/build | High | Review bundled development-server advisories; the consulted editor-launch advisory lists 8.2.4 as its patched release. |
| Undici 7.29.1 | Development, through jsdom | High | Audit flags several issues; the consulted TLS-option advisory is patched in 7.30.1 for this major version. |
| Vitest 5.0.0 and `@vitest/mocker` 5.0.0 | Development/test | Moderate each | The consulted mock-registration advisory is patched in 5.0.3. |

Zod's advisory concerns excessive validation-error allocation. A bounded library probe generated 5,001 issues from 5,000 invalid elements despite a declared maximum of one. The app's OpenUI validation also enforces source, syntax, expansion and render-tree bounds; application out-of-memory exploitation was **not** demonstrated. [Zod advisory](https://github.com/advisories/GHSA-mrhx-c7xc-m3w5), [bounded probe](/home/papan/Desktop/Projects/localLM/docs/audits/harness-2026-11-08/zod-bounded-probe.json), [application bounds](/home/papan/Desktop/Projects/localLM/src/genui/validation.ts:15).

Development-tool advisories have configuration and reachability conditions. Their presence does not establish that the packaged desktop binary exposes the development server or mock interceptor. [Vite editor-launch advisory](https://github.com/advisories/GHSA-r825-w8mm-5pxw), [Undici advisory](https://github.com/advisories/GHSA-2m5v-pch6-74hx), [Vitest advisory](https://github.com/advisories/GHSA-qxg6-89mg-rqv8).

**Long-running execution: present limits and missing guarantees**

| Layer | Present behavior | Consequence |
| --- | --- | --- |
| Parent turn | 64 model rounds; final round reserved for an answer. Probe observed 63 tool rounds and a final no-tools round. | Work requiring more rounds stops; there is no demonstrated automatic durable continuation into another turn. |
| Shared native search/fetch budgets | 16 searches and 16 fetch-family units per turn; batches charge per element. | Research can finalize well before 64 rounds. |
| Child agent | Default 4, maximum 8 model rounds. | Delegation is a bounded subtask, not a durable multi-day worker. |
| Managed shell processes | Default 60 seconds; configured maximum 300 seconds. A 3,600-second request was rejected. | A one-hour build/job cannot stay alive through this managed-process API. The adapter's wait timeout does not extend the process lifetime. |
| Other execution paths | Local Python cap 90 seconds; persistent-terminal sends use the configured shell deadline. The raw UI terminal is a different interaction path. | These are not a tested autonomous hour-long execution service. |
| JSON PTC plans | 8 underlying calls and 60 seconds of execution budget. | Useful bounded batching; human-input budgeting is separate. |
| Model HTTP request | 600-second total client timeout, including body; stream idle guard is 300 seconds. | One slow generation is bounded even while an overall task might last longer. |
| Transient HTTP retry | Up to 3 pre-stream attempts with an 8-second wait budget. Long `Retry-After` stops instead of retrying early. | Correctly avoids aggressive replay, but provides no durable scheduled wake-up after a long outage. |
| App restart | Messages/runs are reconciled to interrupted/cancelled states; research ledgers can pause. Live registries and process handles are in memory. | Persistent history supports review/manual recovery, not continued execution by itself. H01/H06 further weaken recovery truth. |
| Failed model-round recovery | One recovery model round with tools explicitly removed. | It can report from saved tool outcomes without redoing actions; it cannot finish outstanding tool work in that recovery round. |
| Research task time fields | Struct defaults include 1,800 seconds and validation allows up to 7,200. The parent loop does not enforce `elapsed_exhausted()`. | These fields are not proof of either a two-hour execution guarantee or a correctly enforced task deadline. |

Sources: [turn budgets](/home/papan/Desktop/Projects/localLM/src-tauri/src/continuation.rs:109), [process lifetime](/home/papan/Desktop/Projects/localLM/src-tauri/src/process_tools.rs:330), [execution limits](/home/papan/Desktop/Projects/localLM/src-tauri/src/execution.rs:175), [PTC bounds](/home/papan/Desktop/Projects/localLM/src-tauri/src/ptc.rs:22), [HTTP policy](/home/papan/Desktop/Projects/localLM/src-tauri/src/http_client.rs:120), [retry bounds](/home/papan/Desktop/Projects/localLM/src-tauri/src/inference_retry.rs:8), [recovery mode](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:4530).

One additional source-confirmed risk is credential expiry during a long turn. Subscription credentials are refreshed when the turn/backend snapshot is assembled, then the backend carries a fixed access-token string across model rounds. Its 401/403 path ends with an authentication error; there is no per-round refresh hook. If that token expires while a long turn runs, the next request can stop. Expiry during a real account run was not tested, and no particular provider token lifetime is assumed. [Token acquisition](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:1440), [subscription request/error path](/home/papan/Desktop/Projects/localLM/src-tauri/src/inference.rs:832).

**Efficiency and accuracy assessment**

The app has meaningful positive foundations: stable tool ordering and prompt snapshots, volatile context appended after the reusable prefix, tokenizer-request deduplication, measured main-round usage, artifact capture with safe recovery handles, grouped trusted reads, schema-validated tool calls, bounded repairs, uncertainty-aware replay protection, and source-bound verification tools. Existing regressions exercised these paths in the full suite. The 64-round fixture also confirmed that the first and final tool-schema arrays stayed identical.

Those mechanisms address overhead and protocol correctness. They do not establish successful task completion, correct facts, useful code, or superiority over another harness. The standalone response-latency diagnostic uses small synthetic tasks and bypasses much of the integrated application workflow. Its ten runner tests validate the measuring tool, not the application's model performance.

Keep three cache measures separate:

| Measure | What it establishes |
| --- | --- |
| Local token-count cache reuse | Reuses exact preflight counts and reduces counting HTTP requests. It is not reused inference KV state. |
| Provider/runtime cached input tokens | Measures reused prompt processing when the backend reports it. A cache-enabled request is not a measured hit. |
| End-to-end task efficiency | Requires all parent/child attempts, tool work, latency, correctness and cost. H10 currently leaves child usage unmeasured. |

OpenAI's official documentation confirms that inference caching requires a matching rendered prefix and eligible available cache state; keeping a conversation/session alone does not guarantee reuse. Changes to tools or compaction can affect matching. Cache reuse itself does not change output generation into a correctness guarantee. [Official prompt-caching documentation](https://developers.openai.com/api/docs/guides/prompt-caching), [cache diagnostics and usage](https://developers.openai.com/api/docs/guides/prompt-caching/diagnostics).

**Acceptance work needed before stronger claims**

Repair H01–H04 and H08 first, then completion truth and measurement. Update flagged dependencies and restore the unchanged integration gate. Keep the production fixes reviewable and replace diagnostic expectations with real correctness regressions; do not count these diagnostic passes as successful repair tests.

For a comparative pilot, use at least 24 fixed tasks across bug fixes, features, repository investigation, grounded research, long-context continuation and interrupted execution, with three repeats per task/harness. Pin model/version, reasoning/output limits, tool access, workspace starting state, hardware and time/token budgets. Randomize order. Include retries, failures, presentation generation and children in totals. Grade actual deliverables against independent behavioral checks; report all failures and uncertainty. Seventy-two runs per harness are a pilot, not proof of universal superiority.

Report task success first, then model requests and underlying calls per successful task, repeated calls, artifact rereads, time to first useful output, completion median/p95, total tokens and cost. Publish both cached-input fraction `sum(cached input)/sum(input)` and the fraction of requests with reported cache reuse, together with measurement coverage. Separate cold-start, warm repeated-prefix and evolving-conversation workloads. Do not average percentages from differently sized requests or treat missing counters as zeros.

For durability, run actual 1-hour and 2-hour jobs followed by 24-hour and 48-hour soaks in isolated fixtures. Inject process kill and restart, machine suspend/resume, stream disconnects, long rate limits, credential rotation, pending approvals, disk/database failures, compaction, concurrent parents/children and background-job completion after the initiating view closes. Measure memory/disk growth, durable terminal state, preserved requirements, recovery position, and duplicate/uncertain side effects. Require completed work to remain completed and unfinished work to remain visible. Long shell jobs need an execution backend designed to outlive the current five-minute ceiling.

These acceptance runs were **not performed** in this audit. A 64-round synthetic run and restart probes are useful fault coverage, but do not substitute for hours or days of elapsed operation. The evidence supports fixing the documented defects and running those acceptance gates before presenting LocalLM as a reliable multi-day autonomous harness.
