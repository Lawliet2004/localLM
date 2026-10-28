# LocalLM harness simplification — implementation and evidence

Status: implementation candidate under evaluation. **The replacement gate has not passed.** The evaluator has not installed over, launched, or replaced the user’s application. Existing dirty source edits and user conversation storage are preserved.

## Preserved baseline and build provenance

The baseline is the complete initial dirty source tree, not a clean checkout of HEAD. Its archive is `.local/qa/harness-simplification/baseline/source.tar.gz`, SHA-256 `85def99ee76ce660f1da6b2616ecddc560efabe1c1d43a7b17d9bd8cf088db94`, based on HEAD `8d5cec5b3e9ad7a7b26c6cf57bb1faeb02783adb`. The manifest records all original file hashes and dirty paths. No reset, stash, clean, or conversation replay was used.

The baseline evaluator adds only the same feature-gated headless entry point used by the candidate. Every trial gets a separate synthetic workspace, app database, provider configuration and permission record. Request/response captures, resulting file hashes, tool records, terminal states and token reservations remain in `.local/qa/harness-simplification/evaluations/`. Credentials travel through a private pipe and are not written into those records.

| Evaluator | Binary SHA-256 | Evidence |
|---|---|---|
| Dirty baseline | `7956f5142583ba77b56f9634cc785328a047dc46a93dcadea320b1eac0b3599f` | Initial comparison |
| Frozen candidate V7 | `f5ac3d141870112ab0cf642ad6183e26f20767812be411b60ac202bf8903f8f1` | Initial comparison |
| Frozen candidate V8 | `908a79c2d17e0f8f619d3e8a542784c1b451c3af3e5082a0ac9deacccd2815ff` | Three separately labeled remote follow-ups |
| Frozen candidate V11 | `719d9d4964a9e64f12c894189dbfefc14802b7cdfb5ee024ea40ebdc7186f01b` | Stream-error recovery, explicit child resume and restart transactions; 36 remote trials reviewed |
| Frozen candidate V9 | `62a86757d2fa96c14b1e358a078ae01d4574effe0d73e755e66df0b74aba0f23` | Passage/cursor, cutoff and termination fixes |
| Frozen candidate V15 | `bfbf5fc2eef822b72dfe6520e0381a3b001ed38ef249b86b0b784cb96be93bed` | Unicode cursors, shared stream consumption, runtime template validation, early-denial repair; partial comparisons stopped for the observed nonzero-exit bug |
| Frozen candidate V16 | `9f7e08bfe3dd41db829aa86fe381cb040a5247fdf5498b4b8ece365d4ee969e8` | Known process failures, bounded execution output and settled terminal hooks; current comparisons |
| Frozen candidate V17 | `0d653968a743336d067b8a1b09268e786f4f31a07fb0b85d3b74785bddc733ac` | Transactional file aliases, known no-change file failures and shared failure streaks; current comparisons |
| Frozen candidate V18 | `027ee266a4a88028ba6d848e4777b3282fe093b822dc2e4f86fafe4aa4a43db9` | Callable shared driver with typed foreground/child host adapters; current implementation |

These are debug evaluator builds, with the same profile for baseline and candidate. The V11 production release executable is separately identified by SHA-256 `4fb68dbd99c681008ab447bf94185b88a49d984090a3de671aa79468313c4429`; `candidate-v11/release-manifest.json` verifies its source inventory against the frozen evaluator after a successful Tauri build. V7's source inventory is `07aae1fd7920ee93503f2dce0dc488a8f44cf534e9de91afb4a934a93cc1449f`; V8's is `b271deb8cae16a095591a8fb004f3cbe6b6827260680ae55b98d96d39e1201b7`. V11’s source inventory is `e0928dee12b36d12ea89030ab04d905d7012be49d9ffd1bb59c01df6932b2474` and its source archive SHA-256 is `8e19ac7e9661acb690429714c9744d980d8900cfa491af2cee8410c1eb9865bb`. Frozen archives and manifests remain immutable. Later fixes cannot retroactively change a measured V7 result. `scripts/harness-freeze.py` builds before copying, checks input hashes across compilation and archiving, and refuses to overwrite an existing revision.

## Implemented behavior

- The shared engine provides unbounded default scheduling, native provider stream validation, accepted provider-round records, process-wait deadlines, checkpoints and typed terminal outcomes. Inline and detached subagents use one child host implementation. Foreground orchestration lives outside `chat.rs` in `agent_engine/foreground.rs`.
- Productive work has no fixed 64-round parent or child ceiling. Explicit child budgets remain available. Repeated identical failures block that action without disabling unrelated tools. Process completion waits wake from the supervisor rather than requiring repeated model generations.
- Provider clients use a connection deadline, initial-response deadline and stream-idle deadline. Active streams have no three-minute total HTTP timeout. Bounded request and stream recovery continues from committed history. Incomplete tool-call batches never dispatch; partial attempts are journaled separately. Output-cutoff recovery is bounded for both child hosts as well as foreground turns.
- Recognized transient overload/server SSE errors use the same bounded recovery as disconnected streams, without echoing raw provider messages that could contain credentials. Permanent quota and permission errors remain failures.
- Runs carry distinct completion, cancellation, pause, restart, provider failure, budget, permission, uncertain-outcome and blocker causes. Repetition guards and budget exhaustion preserve resumable interruptions. Denied or uncertain tool actions cannot be erased by a later unrelated successful call or a final paragraph.
- One process supervisor executes model shell, Python, command and optional check entry points. Process results persist before completion notification. Restart restores completed evidence and marks in-flight outcomes unknown; it does not relaunch external actions. The user's interactive PTY terminal remains available.
- Explicit child resumption is available through `subagent.resume_from`. It reconstructs committed exchanges from the journal, retains native reasoning, pins the original tool identities and schemas, narrows permissions to the current parent, and requires the original conversation/workspace/model/protocol. Completed commands are not rerun by recovery. Unstarted siblings receive undispatched results; dispatched actions without confirmed results refuse automatic resumption. Explicit round budgets require a new explicit allowance. Startup never launches these actions.
- Startup recovery writes terminal events within its existing SQLite transaction; it no longer attempts a nested transaction. Run-event sequence allocation is a single write statement across connections. Child startup state and listed outcomes now distinguish recoverable checkpoints from uncertain actions.
- SQLite changes are additive: run outcomes, checkpoints and process results coexist with retained conversations, artifacts, credentials, native messages and tool records. Startup recovery records an application-restart cause without asserting that an abandoned action succeeded.
- Chat Completions reasoning content now survives stream assembly, persistence and replay, alongside existing Responses and Claude native state. River AI behavior is not inferred from a DeepSeek model name. The selected compaction plan is persisted and applied once, retaining complete exchanges and a structured checkpoint with requests, constraints, observations, pending work, source references, process handles and archive references.
- Normalized artifact pages retain the original identity, advance a valid cursor and do not create recursive artifact chains. New artifact content uses the canonical six-field `locallm.tool-envelope/1` projection once. Legacy saved results remain decodable. Image-bearing tool results retain their image parts. Artifact persistence errors are visible.
- Bounded workspace results retain committed file hashes independently of artifact checksums. Bounded web results retain document/session handles. Web find/open passages retain usable text and cursors; cursors advance only over text actually shown. Document/PDF windows preserve exact stored text, including long code paragraphs and Unicode, across storage reconstruction.
- The measured Standard catalog advertises 18 tools rather than 64. Core catalogs are capped at 20; there is no alphabetical filler. Authorized skills, MCP and specialist tools use discovery/invocation. Frozen catalogs are invalidated by the tool-contract version. Legacy capability selections do not gain permissions during conversion.
- Removed bespoke database/rehearsal, debugger, profiling, benchmarking, HTTP/OpenAPI and impact-analysis tools, PTC, `workflow_run`, `ralph_run`, unused registrations and the corresponding development settings. Removed direct SQLx dependencies and their unused dependency chain.
- Web research now performs agent-requested search and document retrieval. Hidden planning, synthesis, model verification, embedding and reranking paths, unused configuration and obsolete standalone research loops were removed. Extraction, caching, source references and network protections remain.
- Chat interruption messages explain restart checkpoints and uncertain action outcomes using the typed termination cause. This copy change was checked with component tests; no new browser or native screenshot acceptance is claimed.
- Command evidence is captured from ordinary execution and process results. Command success, parsed test reports and unverified claims remain distinct. No mandatory `check_run` rerun, report-format rewrite or automatic todo completion is imposed. Commentary and final answers are persisted and rendered separately. Presentation configuration remains in Settings.

## V12 follow-up

The V11 trials exposed a URL-read path that ignored `textOffset` and repeated the beginning of a document. URL reads now forward passage/page/section/text cursors into the shipped worker and use the same document-window implementation as saved-handle reads. A cache-seeded production dispatcher regression covers both `web_read` and `web_open`, including stable document identity and parity with saved-handle reads.

Foreground and child streams now share `RoundContent`: text reasoning markers, native reasoning deltas, usage, finish reasons, output bounds and streamed tool-call accumulation have one implementation. The existing incremental reasoning filter lives in the engine, with a compatibility re-export for callers. Both child hosts are tested with reasoning markers in their final response.

The old research-ledger budget type and stop counters are deleted. Saved budget JSON remains readable and round-trippable; new ledgers have no independent budget or automatic plan/search/read/verify/answer checklist. The shared engine and explicit user budgets control execution.

V12 evaluator SHA-256: `ab375ace38d29f438ea7f82696bb2d9809ae27ed3225be8383838c5f56610b29`; source inventory: `f0b39eb40fcae8152d95b4ac934227b4a400e936cba918453edcbd6744fcc3e2`; source archive: `7c7a0a1d1bd322654172d7882ed590bf6e851bae44edb36cba6015ebeaf5db86`. Its focused replay-contract trials yielded one source-reviewed completion in three, separately from V11 outcomes. Validation so far: 958 Rust library tests passed, zero failed, 17 ignored; Rust check, warnings-denied Clippy, typecheck, all nine explicit loopback tests and five shipped-worker tests passed. The frontend suite passed 836 tests with one skipped, and all 19 transport fixtures passed. The V12 production build was deliberately stopped before source edits for a second cursor defect; it is not a successful release build. `tauri-production-v12-cancelled.json` records the verified evaluator-owned build tree stopped; user application processes were not terminated.

The V12 source estimate removes 12,303 production lines (537,256 bytes) and, separately, 3,815 test lines (149,802 bytes) against the preserved baseline. Source counting excludes generated assets and uses the documented lexical test classification. No dependency changes since V11.

## V13–V15 follow-up

Initial web extracts now return document text cursors. Artifact bounding reserves room for usable text and updates the cursor to the exact UTF-16 length actually shown; serialized artifact byte counts are never document offsets. A production-path test follows every bounded cursor across a Unicode extraction boundary and reconstructs the source exactly once through both reader aliases.

Foreground and child runs now consume `ModelRound`, which owns provider decoding, error classification, tool-call accumulation, reasoning filtering and the final marker flush. Rendering and persistence receive interpreted deltas. The unreachable tool-free planning branch was deleted; plan approval remains in dispatch. This consolidates stream execution further, but does not finish the remaining host orchestration refactor.

The installed Gemma template's known mapped-uppercase expression is expanded into an equivalent loop in a scoped temporary file. Matching uses exact template content, with no model-name heuristic; GGUF bytes remain unchanged. A render-only startup request exercises nullable array-item schemas and replayed tool messages without generating a response or invoking a tool. Rendering failure stops the child and never reports ready. The installed runtime/model passed this actual launch check (`installed-runtime-template-v14.log`); a subprocess fixture additionally checks cleanup, authentication and failure behavior. The evaluation template is byte-identical to this product rewrite.

General response guidance no longer demands terminal-log formatting. Capability provenance JSON remains available in diagnostic context events and is omitted from the user draft. Workspace guidance asks for task-relevant verification without a fixed execution or report-format ritual.

V15 repairs early permission-rule and capability denials that bypassed outcome tracking. A later unrelated success or final paragraph cannot mark those runs completed. An undispatched blocked repeat reports no new side effects, rather than assigning the earlier attempt an invented uncertain outcome. Failed results with explicit uncertain side effects retain that uncertainty; successful legacy results are not treated as failed actions. Production HTTP regressions exercise both early-denial paths and repeated invalid arguments.

V13 and V14 production builds were deliberately stopped for these further source repairs. They are not successful release builds. Their frozen evaluator manifests and cancellation audits are retained. One V14 remote trial began two requests during coordinator cancellation; it is explicitly recorded as interrupted evaluation evidence, with all usage/reservations still charged, and has no quality result. V15 source inventory is `7062d91734ae4f3cb5f83608b367916973c925ac68df264a1bdfb454b36261dc`, source archive `79d2661004c6e24127200ebb01ddcb5cbdb60b3609e782731348eb1503628bed`. Its production build passed; the verified release executable is `4e7248b43c846d80ebd3f56e6b4f505d64b356d415dcae52854b48b7e78a0015`, preserved without installation or launch.

V15 live evaluation exposed a conservative but incorrect attribution: an observed nonzero process exit inherited generic unknown side effects, making an otherwise repaired task end as `outcome_unknown`. V16 records completed, fully drained process results as applied effects, including exit failures. It retains unknown results for interrupted, incomplete or unconfirmed execution. The shell adapter preserves this certainty, and successful bounded output becomes a partial envelope instead of a malformed-success error. Actual command, restart, foreground and both child-host regressions cover the repair.

Terminal status and finish hooks now settle once after orchestration and presentation. A final paragraph following a denial cannot publish an early completed status. V15 comparisons stopped after seven remote and two local completed trials; their active remote/local trials retain interruption evidence and all reservations remain charged. They are not a complete quality comparison.

V16 source inventory is `7b40d909b4f324afe9afb5a9f2dca495e50580b2d3ea42ff9170012bb4900690`, source archive `4b3073cd26db194d441b670258735a625c65c9b36a9317d7116a70d2a4a82ccf`. Validation passed 961 Rust library tests, zero failed, 20 ignored, check, warnings-denied Clippy and all eleven explicit loopback regressions. A fresh frontend run passed 836 tests with one skipped, typecheck, 19 transport fixtures and five shipped-worker tests. The first V16 transport invocation used an incorrect npm script name; its failed invocation log is retained, and the correctly named command passed separately. The production build passed; its verified release SHA-256 is `8d7a76cd9e3e69f8dc99ebe73a37e50f3da204e043207ddded1b6683b4cfddee`, preserved without installation or launch. The real 61-second process regression also passed on V16.

Four remote and two local V16 trials finished before a second attribution bug stopped the comparison. A missing-file read and pre-creation failure inherited generic unknown side effects. Even after subsequent successful repair, this incorrectly poisoned the terminal run and blocked an identical creation after its missing directory had been created. The completed trials remain recorded; two active trials retain interruption evidence and charged reservations. No later revision receives credit for these earlier trials.

## V17 file execution consolidation

`create_file` now uses the existing transactional patch writer. `edit_file` reads and matches as before, then commits the replacement through the same hash-pinned writer; it no longer truncates and rewrites the existing file in place. Existing no-overwrite, scope, hash, matching, line-ending and formatting behavior remains covered. Simple alias results retain their path, hash and edit diff without duplicating a full creation diff. Listing hides the transaction journal and other paths excluded by the existing filesystem policy.

The common workspace model boundary returns known no-change failures for reads and transaction preflight. Explicit commit, rollback and unknown transaction results pass through unchanged; recovery and directory mutation errors remain conservative. The foreground, inline child and detached child use this same boundary. A real six-round regression exercises a missing-file read, failed creation, parent-directory creation, an identical successful retry and a hash-pinned edit through all three hosts.

`ActionFailures` now owns the repeated-failure streak for foreground and children. Two identical failures block their action, a changed failure starts a new streak, success clears that action's failures and unrelated tools remain available. Suppressed child calls report no new effects while retaining the original uncertain result in history. The initial workspace suite caught the transaction journal appearing in `list_files`; the failed log is retained and the implementation was fixed without weakening the assertion.

V17 validation passed 964 library tests with zero failures and 20 ignored, check, warnings-denied Clippy, all eleven explicit recovery regressions and the production build. Source inventory SHA-256 is `c43110bb547304ee4541c2cdc8a7f6ea6b6eb70a0ca35557c67831dab56a4776`, archive `8d0c6f61474fdbeed2d9caeb814c51e55a8e2f4275ff49970232eb21875a4c3d`, production release `bc4b98a5f5af063c92e6fe601d9f03ab6f2745bb85b8334bfcf56cc87aa61525`. Frozen input parity was verified after the build. The release was then launched only on an owned isolated Xvfb/D-Bus display with fresh QA storage. Native transcript acceptance passed light/dark and desktop/narrow layouts, file-change disclosure and restart restoration; screenshots were inspected. The installed application and user storage were not replaced. Frontend and worker sources are unchanged from their fresh successful V16 validation.

Five isolated evaluator-accounting tests passed, covering concurrent reservations, per-trial versus combined limits, missing/invalid usage and conservative charges. New evaluators capture their script hash once at startup; a separate V17 attribution correction preserves its loaded-script identity when its source file subsequently changed. The original fixture records remain untouched.

## V18 callable execution core

The macro-expanded scheduling loops are replaced by `drive_session` and a typed `SessionHost` boundary. Foreground state and the shared inline/detached child state now implement one-round adapters; neither host owns the outer model-generation loop. The common driver owns scheduling and explicit round limits, and propagates host blockers without replaying committed actions. The shared `ModelRound` continues to own provider decoding and acceptance. Context, authority, UI delivery and storage remain explicit host responsibilities. Additional sharing of their context/dispatch phases is still pending; this is not a claim that every host-specific branch has disappeared.

Focused validation passed the engine tests, all 25 child-host tests and eleven explicit foreground recovery tests. The full V18 suite passed 965 tests, zero failed, 20 ignored, plus check, warnings-denied Clippy and a successful production build. Source inventory SHA-256 is `2c1aaa8e9ad3cea81d0658286b4afb0a15d8f6c2c713e8fbd3f1d363d93f3a63`, source archive `18b8870c3b31d1d023b08b8c5b15555d3219b7dc04725d219a53699d407f5b74`, production release `73ebb586236333ae6d0b618f74dbcb57d0108991c3411fa0653bc2dcf9779a6d`. Frontend and worker input parity with the tested V17/V16 inputs was verified.

V18's packaged native visual-response fixture also passed on isolated Xvfb/D-Bus storage. It exercised interactive quiz scoring, binary-search stepping, SQLite state, restart restoration, rich components and safe fallback for malformed/interrupted visual output. Screenshots were inspected. This was an authored deterministic fixture, with no model configured, and establishes renderer behavior rather than model generation quality. The first launcher invocation tried to hash deleted tracked files; its failure is retained. The launcher's existing source-snapshot mode enumerated the current filesystem and passed, with source and binary hashes unchanged across the run. All 42 regression-index references were checked against compiled Rust tests or their frontend source test names; two stale catalog module paths were corrected.

The combined remote budget refused the next V17 request at 19,927,181 charged/reserved tokens. Sixteen V17 remote trials have quality outcomes; the next trial is incomplete and excluded from quality denominators. No V18 remote quality comparison is claimed. The V17 local evaluator exited after four graded trials and left its active research trial without a terminal grade; the interruption cause was not observed. Its owned server was stopped, the partial evidence remains explicit, and a fresh V18 local matrix is running serially with separate accounting and owned-server cleanup. The replacement gate remains unpassed.

## Deterministic validation

The initial dirty baseline actually ran 1,018 Rust library tests and 891 frontend tests successfully, with 24 and one skipped respectively. The older counts in AGENTS.md are historical checkpoints. Removed-feature tests are reported separately from production reduction; a smaller test count is not a quality claim.

Current validation logs live under `.local/qa/harness-simplification/`:

| Check | Recorded evidence |
|---|---|
| Rust library | `rust-tests-v18.log`: 965 passed, zero failed, 20 ignored |
| Rust check | `check-v18.log`: passed |
| Clippy, all library targets, warnings denied | `clippy-v18-final.log`: passed |
| Explicit loopback recovery regressions | `permitted-host-v18.log`: eleven passed |
| Frontend tests and registration contract | `frontend-v16.log`: 836 passed, one skipped |
| TypeScript check | `typecheck-v16.log`: passed |
| Network/render transport fixtures | `web-transport-v16-final.log`: 19 passed |
| Shipped search worker HTTP fixtures | `grounding-v16.log`: five passed |
| Frontend production build | `tauri-production-v18.log`: passed (Tauri beforeBuildCommand) |
| Tauri production executable | `tauri-production-v18.log`: passed, `--no-bundle`; isolated QA launch only |
| Native transcript/restoration fixture | `native-v17/acceptance.json`: passed; identical frontend inputs in V18 |
| Native visual-response/restoration fixture | `genui-native-v18/acceptance.json`: passed; deterministic authored components |

Only completed logs with successful exit/results count as passing. Optional Windows-runtime, live-download, installed-language-server, desktop and browser tests are not claimed to pass. No browser localhost preview or alternate preview route was used. The real 61-second command regression passed separately; the active-stream regression advances controlled Tokio time beyond three minutes. The productive-run regression performs 67 distinct real scoped writes before its final answer.

Cold lazy imports exposed two frontend test races. Those tests now wait for actual dynamic imports before asserting rendered content. A separate five-second settings-test timeout under concurrent load passed in isolation; the V9 final frontend run used one worker; V10 and V11 passed with two workers without increasing test timeouts. Earlier unsuccessful or interrupted validation logs are retained.

The sanitized production regressions are indexed in [regressions.json](../evaluations/harness/regressions.json). The source-counting method and separate production/test byte totals are recorded in `.local/qa/harness-simplification/source-reduction.json`. It counts handwritten Rust/TypeScript/CSS and estimates Rust test-only items lexically; generated assets, docs and evaluation scripts are excluded. The frozen V18 estimate removes 12,022 production lines (528,458 bytes), separately from 3,192 test lines (116,954 bytes), against the preserved dirty baseline. Dependency reduction is 40 locked packages and three direct SQLx dependencies, with no added packages.

## Live outcomes and remaining gate

All 72 planned remote trials for the frozen V7 comparison finished. Completion requires a resulting deliverable, its checks/review, and a successful terminal run.

| Remote workload | Baseline completions / 12 | V7 completions / 12 |
|---|---:|---:|
| Coding | 8 | 9 |
| Research | 0 | 5 |
| Conversation/document/visual | 10 | 9 |
| Total | 18 / 36 | 23 / 36 |

The aggregate improved, but the document workload regressed, so the no-workload-regression gate fails. Errors include incorrect optional explanatory claims in one data document and two schedules. Some coding runs saved working files but exhausted the trial budget; these are not counted as completed. Other runs exhausted output tokens before saving any file.

V8 follow-ups are separate: the cache-reference research task produced a source-reviewed document; the game task exhausted output tokens without a saved game; the reasoning-replay research task exhausted its evaluation allowance without a saved answer. V9’s reasoning-replay research follow-up exhausted its evaluation allowance without a saved answer. Its trace exercises the repaired passage cursors, which does not establish an outcome improvement. V11’s separate 36-trial remote comparison is complete and reviewed: coding 9/12, research 9/12, document/visual 10/12, totaling 28/36 against the baseline’s 18/36. This remote subset improves aggregate completion without a workload regression. The game trials and two reasoning-replay trials still exhaust the evaluation allowance; the third replay trial saves an inaccurate answer and ends unsuccessfully. One schedule and one data answer add incorrect optional claims. These failures remain failures. V11 has no completed local comparison, and subsequent revisions cannot inherit its measured outcomes.

All 72 baseline/V7 local trials finished with zero required-deliverable passes for either build. The runs used the same installed Gemma-4-E4B Q2_K_XL server and equivalent inline-loop template adaptation. The product now applies that narrow content-based compatibility fix and validates rendering during startup. These later implementation changes do not alter the V7 measurements. Builds and tests overlap these trials, so latency is descriptive rather than a controlled performance measurement.

Full Access commands are not OS-sandboxed. One baseline research trial copied two scratch documents into the repository root rather than its synthetic workspace. Their exact command provenance and hashes were recorded in that trial's `outside-workspace-changes.json`; the files were preserved in its evidence directory. No original baseline files were removed. Future fixture containment needs to account for this behavior and inherited ancestor instructions.

Every remote request reserves against one shared 20-million input-plus-output token ledger, including excluded diagnostic attempts. Missing usage consumes the whole reservation. Local accounting is separate. An explicit fixture-budget denial is not evidence of a River AI outage. Check [RESULTS.md](../evaluations/harness/RESULTS.md) for the current counts, charges and review status. Incomplete trials remain incomplete evidence.

Game logic was exercised with the saved JavaScript and stubbed canvas/clocks. SVGs were rendered and visually inspected as static artifacts. These checks do not establish real browser gameplay, timing, accessibility or full application UI acceptance. Factual/false-completion dimensions without individual review remain ungraded rather than silently passing.

## Work still required

1. Finish sharing the remaining host-specific context and dispatch phases. V18 provides one callable outer driver and shared stream acceptance, recovery policy, outcomes and child hosts; foreground policy/dispatch still contain separate adapter logic.
2. Complete the current frozen candidate comparisons and required reviews. The baseline/V7 matrix finished; both local builds failed all 36 tasks. V17 remote evaluation stopped at the combined token cap, and V18's local matrix is running. Earlier revisions cannot establish current-build quality across both models; no additional remote allowance has been assumed.
3. Address remaining local-model instruction-following and deliverable failures using measured outcomes. Template compatibility now passes actual product launch/render validation; it does not establish task quality.
4. Extend the passing isolated native transcript/visual fixtures with actual-model visual generation evidence where needed. These deterministic renderer checks are complete, but model task quality and the no-workload-regression gate remain unestablished. The browser-preview restriction remains in force.
