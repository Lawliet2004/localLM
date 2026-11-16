# LocalLM harness acceptance revalidation

9 November 2026 · Linux x86-64 · HEAD `609209e7` plus the existing dirty tree

Four malformed provider streams reached a real workspace write in the starting tree. The registered-turn regressions now reject all four before dispatch. The finished build passed PostgreSQL and MySQL acceptance on disposable servers, including MySQL migration failure and cleanup. Browser and packaged desktop flows remain unverified.

**The Claude Code experiment uses the same local MiniCPM5-2B Q4_K_M model for both harnesses.** Its accuracy, request, token and elapsed-time measurements are scoped to the pinned model, endpoints, tools, and two read tasks. Production Claude models were outside the experiment.

## Findings and closure

P1 covers side effects, ownership or task constraint loss. P2 covers correctness, recovery, measurement or integration. Earlier findings retain their historical reproductions. Status in the tables refers to the paths covered by this revalidation. [Original audit](/home/papan/Desktop/Projects/localLM/docs/HARNESS_RELIABILITY_AUDIT_2026-11-08.md) · [Original fixes](/home/papan/Desktop/Projects/localLM/docs/HARNESS_RELIABILITY_FIX_RESULTS_2026-11-08.md) · [Earlier continuation](/home/papan/Desktop/Projects/localLM/docs/HARNESS_ACCEPTANCE_REMEDIATION_2026-11-09.md)

| New finding | Severity and state | Reproduction, cause and repair |
| --- | --- | --- |
| R01 · Mixed Chat Completions choices fabricated a call | P1 · Fixed | Send the path prefix in choice 0 and the rest of the write arguments in choice 1, then end with `tool_calls` and `[DONE]`. The accumulator used the first array position while tool indices belonged to individual choices. The shared parser now rejects multiple choices or an explicit nonzero choice index before accepting fragments. Single-choice streams with omitted indices and usage-only tails retain coverage. |
| R02 · An unclosed Claude thinking block permitted a write | P1 · Fixed | Start a thinking block, close a valid sibling tool block, then send `tool_use` and `message_stop`. The old closure check covered tools only, allowing execution alongside unfinished native replay context. The native adapter now tracks recognized text, thinking, redacted thinking and tool block closure together, rejecting deltas after closure and content changes after the finish reason. |
| R03 · Responses could reinterpret a refusal as ordinary text | P1 · Fixed | Stream `response.refusal.delta`, then present that text as `output_text` in a completed response with a write. Both part kinds shared the same accumulator. Reconciliation now preserves part kind and checks that all streamed parts survive into the final item. A relabelled refusal invalidates the round. |
| R04 · Responses accepted arguments appended after `.done` | P1 · Fixed | Emit `response.function_call_arguments.done` with a draft write, extend it with a later delta, then complete the batch. The old suffix merge let finalized arguments change. The new state accepts an identical final confirmation and rejects post-finalization changes. The existing `max_output_tokens` recovery test still drops the entire truncated batch and runs a fresh read. |
| R05 · Message submission exposed a stale presentation write | P2 · Fixed | The IPC command accepted a mode and stored it before turn registration. The composer had stopped supplying it, but a submission with the stale field could change a saved setting even if admission failed. Removed it from the invoke payload and command. Saved-request recovery retains its ID and image payload, and presentation preflight still reads the effective setting. |
| R06 · A comparison timeout hid settled tool work | P2 · Fixed in the recorder | The warm Claude four-file trial hit 240 seconds. The timeout branch previously discarded the operation counts because the CLI never emitted a terminal result. The runner now parses complete events from a timed-out trace, preserves settled counts as lower bounds and leaves the missing denial summary unknown. A regression covers a partial last line and a pending call. Recounting the real trace recovered 247 recognized tool IDs, 244 settled operations and 120 successes. |

R01–R04 each failed the file-absence assertion on the starting parser, after creating `unexpected.txt` in an isolated workspace. On the finished tree, each case checks the protocol-specific error, the expected HTTP route, one model request, a failed saved assistant, zero tool rows and an absent file. The pre-existing unclosed Claude tool fixture also passed. [Before repair](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/protocol-before-fix.txt) · [After repair](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/registered-protocol.txt)

Changed source files. [Shared stream](/home/papan/Desktop/Projects/localLM/src-tauri/src/sse.rs:119) · [Claude adapter](/home/papan/Desktop/Projects/localLM/src-tauri/src/claude_adapter.rs:26) · [Responses adapter](/home/papan/Desktop/Projects/localLM/src-tauri/src/responses_adapter.rs:278) · [Submission IPC](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:4437) · [Comparison recorder](/home/papan/Desktop/Projects/localLM/scripts/run-harness-comparison.py:152).

| Inherited finding | Severity and state | Repair retained and coverage rerun |
| --- | --- | --- |
| H01 · Interrupted or failed children left ownership unsettled | P1 · Fixed in scoped fixtures | Aborted foreground work, failed background initialization and startup reconciliation exercise the admission lease, durable dispatch records and explicit unknown outcomes. |
| H02 · Descendants survived parent/app exit | P1 · Fixed on Linux fixture paths | The guardian owns a private process group. Tests kill the owner or end the leader and check descendants, registry drop and stop acknowledgement. |
| H03 · Multipart constraints disappeared in a handoff | P1 · Fixed | Checkpoint assembly preserves original text and archive references. The repeated active-exchange regression resolves the original attachment indices and preserves the unfinished parallel round. |
| H04 · Child requests missed workspace instructions | P1 · Fixed | Foreground and detached prompt assembly inherit the applicable project instruction chain and selected skills. Captured prompts verify the rule sentinel. |
| H05 · Budget exit or pending work looked complete | P1 · Fixed for exercised states | The 64-request case preserves tool order, disables tools in the last request and reports partial. Graph tasks and unknown ledger operations also keep the partial outcome. |
| H06 · Completed ledgers reopened as running or paused | P2 · Fixed | Run binding and reconciliation retain completed ledgers, including the 55 historical rows fixture. The resumed turn settles the original task ID. |
| H07 · Reasoning-only child answers counted as success | P2 · Fixed | The child rejects empty visible output and records the usage received on the failed round. |
| H08 · Background work escaped admission limits | P1 · Fixed per application database in one process | Foreground and detached children share eight permits. The ninth request is refused and settlement releases a slot. |
| H09 · Pure programmatic reads invalidated verification | P2 · Fixed | The typed effect path keeps pure reads verification-neutral and conservatively handles writes, hooks and opaque work. The controlled batch used two requests and one outer call versus three and two directly. Both paths read two files and returned the same answer. |
| H10 · Child cache and usage counters were lost | P2 · Fixed | Child measurement, late parent settlement and diagnostic display preserve reported usage and represent missing counters as unknown. |
| H11 · Quoted handoff text replaced trusted state | P2 · Fixed | Internal handoff fields carry provenance and are stripped before provider transmission. Literal examples remain ordinary message text. |
| H12 · npm advisories on the prior lockfile | P2 · Fixed at scan time | The updated lockfile passed build and test gates and the registry scan reported zero vulnerabilities. |
| Concurrent SQLite settlement and event sequencing | P2 · Fixed in fixtures | Writer reservation and atomic sequence allocation preserve 160 messages and 160 run events under concurrency. |
| Subscription token rotation | P2 · Fixed in fixtures | The next model request resolves the rotated credential, rejects an account switch and redacts both token values. The fixture uses isolated credentials and loopback. |
| C01 · MySQL 8.4 metadata arrived as binary or uppercase | P2 · Fixed | Decode text at known metadata boundaries and look up column names case-insensitively. The real fixture sees names and constraint definitions while ordinary binary query values retain explicit omission. |
| C02 · Fresh scratch user failed native authentication | P1 · Fixed on pinned MySQL | Explicit TLS disable requests the server RSA public key for the first `caching_sha2_password` login. The runner verifies native tools ignore a deliberately invalid default option file. |
| C03 · MySQL QUICK checks failed through prepared statements | P1 · Fixed on pinned MySQL | Error 1295 was reproduced with the prepared protocol. The text protocol sends the bounded check for the quoted scratch table. Acceptance confirms positive status and absence of scratch roles and schemas. |
| C04 · Unclosed Claude tool and late compatible deltas | P1 · Fixed | Incomplete Claude tool blocks and compatible data that contradicts a finish reason fail before dispatch. The inherited registered Claude test and shared bytewise parser tests passed. |
| C05 · Refusal text was lost or dispatched with a tool | P1 · Fixed | Compatible refusal text is visible and durable, including usage-only tails. Native Responses rejects refusals mixed with execution. R03 covers a separate kind-reconciliation path. |
| C06 · Active compaction used wrong attachment coordinates | P1 · Fixed | The first archive includes the real multipart request. Repeated compaction keeps its original references and cuts only a settled prefix with exactly one result per call ID. Small retention windows and unfinished parallel exchanges are covered. |
| C07 · Literal legacy checkpoint prefix overwrote a real request | P1 · Fixed | Restore targets the internally marked placeholder. Reopen and recompact retain the literal user turn. |
| C08 · Resumed segments reissued exact side effects | P1 · Fixed for linked audited parent and program calls | Saved run membership seeds the shared signature guard. Staging survives a failed preflight and adoption consumes the checkpoint. The aged SQLite test performs two explicit resumes and rejects four exact write attempts. |
| C09 · Process stop returned a running snapshot | P2 · Fixed on Linux | The supervisor publishes the terminal state and bounded drained output before acknowledging stop. The acknowledgement and process cleanup regressions passed. |
| C10 · Truncated Responses batches skipped bounded recovery | P2 · Fixed | Incomplete function items stay drafts until the terminal event. Output-limit recovery discards the sibling write and then executes one real read. |
| C11 · Pause before dispatch created false uncertainty | P2 · Fixed | Pausing before dispatch records pending work without creating a running operation. The pause-settle-recover test verifies no unknown operation. |
| LSP readiness and transient edits | P2 · Fixed in local fixtures | Readiness polling and bounded `ContentModified` retries are restricted to unchanged pure reads. Rust and TypeScript navigation and semantic-impact checks passed on real servers. |

## Databases

Both engines passed twice during this revalidation, first on the frozen starting build and then on the finished build. Each attempt used new UUID-named containers bound to loopback and local digest-pinned images. The runner verified container absence with a successful Docker query.

PostgreSQL exercised read-only access, source preservation, disposable migration and interruption cleanup. MySQL exercised an independently SELECT-only source account and a generated role scoped to the scratch schema. The successful rehearsal returns an enforced `positive_id` CHECK, a primary key, the new column and `CHECK TABLE QUICK` status `OK`. The failing rehearsal runs real scratch DDL and attempts to insert an invalid CHECK value. Both rehearsals leave the original row and two-column source schema intact. The fixture queries for absence of both generated roles and schemas and repeats cleanup. An ordinary `CAST(... AS BINARY)` query reports explicit omission. [Pinned images and cleanup](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/database-acceptance.json) · [MySQL evidence](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/mysql-acceptance.txt) · [PostgreSQL evidence](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/postgres-acceptance.txt)

The native arguments map explicit TLS disable to `DISABLED` with `--get-server-public-key`, require to `REQUIRED`, and identity verification to `VERIFY_IDENTITY`. Oracle documents RSA password exchange with a server-provided public key for a fresh caching SHA-2 account over a plaintext TCP session. The real fixture covers disable. The require and verify modes were inspected in source. [MySQL caching SHA-2 authentication](https://dev.mysql.com/doc/refman/8.4/en/caching-sha2-pluggable-authentication.html) · [TLS connection options](https://dev.mysql.com/doc/refman/8.4/en/connection-options.html) · [mysqldump client options](https://dev.mysql.com/doc/refman/8.4/en/mysqldump.html#option_mysqldump_get-server-public-key)

## Protocol, harness and application coverage

| Boundary | Observed coverage |
| --- | --- |
| Chat Completions | Shared bytewise SSE decoding covers split UTF-8 and JSON, CR/LF/CRLF, BOM, comments, multiline data and event size bounds. Registered turns cover real writes after bounded recovery, refusal persistence, usage, dropped streams, invalid arguments, and Stop during draft streaming. R01 prevents cross-choice tool assembly. |
| Responses | Native function IDs, opaque reasoning and saved results replay through a loopback registered parent. Missing results are explicit uncertainty. Output-limit recovery skips the truncated batch. R03 and R04 check terminal reconciliation. |
| Claude Messages | Fragmented JSON, dense tool slots over native block indices, signed and redacted reasoning, usage, and batched results retain positive coverage. Registered fixtures reject both incomplete tool blocks and unfinished thinking siblings. |
| Text calls and programmatic dispatch | Owned local-template fallback parses complete bare tool markup and keeps fenced examples inert. Typed output contracts and the `locallm.tool-envelope/1` boundary remain enforced. Programmatic and mixed model/program calls retain canonical audits, bounded artifacts and lifecycle handles. |
| Tool ordering, dispatch and retry | The 64-request fixture and captured same-model requests check stable tool definitions. Discovery advertises a bounded catalog. Live permission changes are checked before actual dispatch, including grouped reads and nested calls. Loopback retry fixtures exercise rate limits, `Retry-After`, cancellation during backoff/headers/body, and disconnects after visible output. |
| Run ownership and replay | Separate queue slots, child admission, launch failures, cancellation, startup reconciliation, and exact-action guarding all have backend coverage. A paused call that never began stays pending. Failed model-round recovery reads durable tool results with tools disabled. |
| Frontend and IPC | The registered command name check and React suites cover Settings-only presentation, saved requests including attachments, response metrics and lifecycle rendering. A backend test reattaches to a real pending approval, and frontend mocks exercise that recovered snapshot. Typed MCP forms, per-call close IDs and detached authority restrictions have backend and dialog coverage. |
| Linux process lifecycle | Actual process groups, owner exit, graceful stop and bounded output finalization were exercised. A real Node inspector capture passed. |

The official protocol references describe the single-choice compatible stream, Claude block stops, and Responses argument completion used in the new validation. [Chat Completions streaming](https://developers.openai.com/api/reference/resources/chat/subresources/completions/streaming-events) · [Claude streaming messages](https://platform.claude.com/docs/en/build-with-claude/streaming) · [Responses streaming events](https://developers.openai.com/api/reference/resources/responses/streaming-events)

## Endurance and recovery

| Build and requested duration | Actual fixture time | Wall time | Reads / requests | Compactions | RSS start / peak sample / end |
| --- | ---: | ---: | ---: | ---: | ---: |
| Starting, 1 h | 3,600.09 s | 3,600.26 s | 24 / 25 | 24 | 69.3/86.3/36.1 MiB |
| Starting, 2 h | 7,200.06 s | 7,200.18 s | 24 / 25 | 24 | 63.4/82.2/41.7 MiB |
| Finished, 1 h | 3,600.09 s | 3,600.32 s | 24 / 25 | 24 | 70.1/92.6/37.9 MiB |
| Finished, 2 h | 7,200.07 s | 7,200.23 s | 24 / 25 | 24 | 69.8/92.6/44.5 MiB |

Both finished-build runs passed on 9 November at 10:56:00–11:56:00 and 10:56:00–12:56:00 IST, with separate temporary workspaces. Each retained the protected file, completed the run and ledger, reported a non-partial complete turn, and reopened with the canonical history and all 24 compaction artifacts. All four scratch directories were removed. The runs had no cancellation, crash, or restart during the active turn. The earlier attempt stopped near 2,652 seconds and is excluded from these completed runs. [Elapsed summary](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/endurance-summary.json) · [One hour](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/endurance-one-hour.txt) · [Two hours](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/endurance-two-hours.txt)

Each full run is a registered read-only turn with paced scripted inference, 24 real file reads, 25 model requests and an immutable protected file. The loopback server sends heartbeats no more than about 30 seconds apart during the paced wait. The fixture requires actual file contents, no active registered run, automatic compaction, terminal ledger/run/turn states, a non-partial outcome, every archived artifact, and canonical history after opening SQLite again. Memory is sampled process RSS on a shared host.

The separate three-day timestamp fixture ages a persisted checkpoint and really reopens SQLite. A completed write and an uncertain audit survive. Two explicit registered resumes keep one ledger and reject four exact write attempts while leaving uncertainty visible. The timestamp age measures recovery of old state. [Aged checkpoint, turn cap and controlled batching](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/restart-and-batching.txt)

| Capability | Evidence and limit |
| --- | --- |
| Remaining active while the app is running | See the elapsed fixture results. The parent has a 64-request cap with a reserved final round. Child turns default to four and cap at eight. Managed shell work defaults to 60 seconds and caps at 300 seconds per process. Model HTTP requests have a 600-second client timeout and a 300-second stream idle guard. |
| Resuming after interruption | Durable reconciliation and explicit bounded segments are exercised, including older checkpoints and recovery from saved tool results. The write guard covers exact linked audited signatures. Altered commands, separate tool identities, and legacy history without run membership require separate evidence. |
| Continuing autonomously across restarts | Open. Startup marks interrupted work and supports explicit resume. Durable scheduling after app exit or an extended outage remains open. |
| Accurately finishing representative multi-day work | Unverified. A representative elapsed workload with independent outcome checks, outage and restart injection, resource tracking, and reconciliation of writes remains to be run. |

## Claude Code comparison

Claude Code 2.1.319 used bare, noninteractive, no-session-persistence mode. Both harnesses used llama.cpp b10855 commit `7d701b592` on Vulkan1, an RTX 2050 with one slot and a 32,768-token context. The generation configuration used 1,024 output tokens, temperature 0, top-p 1, seed 0 and reasoning off. The GGUF is `openbmb/MiniCPM5-2B-GGUF` revision `2079a22f3beaa4e306449978533478fe0522f4b3`, SHA-256 `ec2d5801640099e97d8d7e8003ad4d81f336e757811f03a26173dddf386602fd`. The loopback recorder forwards LocalLM Chat Completions and Claude native Messages bodies unchanged.

The tasks are a single-file code lookup and a four-file JSON join plus sum. Three fixture seeds, 20261108 through 20261110, each have a cold run and immediate warm repeat per harness and task. The runner alternates harness order, erases the model slot before each cold run with the RAM prompt cache disabled, and restores byte-identical fixture files. Golden objects live outside the model workspace, and file data and metadata changes fail the run. Both harnesses have a 64-round/turn limit and a 240-second process deadline. LocalLM reserves a final model round.

The runner configures corresponding read, glob, search, create and edit permissions. LocalLM uses the Chat preset, workspace source, text presentation, and explicit denial outside five named rules. Claude is configured with `Read,Glob,Grep,Write,Edit`. Actual schemas and permission outcomes are recorded below. The prior contaminated workspace pass and the pass with unequal limits and permissions are retained as diagnostic evidence outside this comparison.

Ten grader examples covered bare JSON, wrong values and types, extra and duplicate keys, fences and surrounding prose. Strict success requires an exact single JSON object, correct typed values, terminal completion, successful tool evidence, unchanged inputs, and no permission denial or setup violation. Factual matches allow a parsed fenced object or extra fields. All attempted runs, including caps and errors, count toward the tables. Cache fractions use summed cached input divided by summed input. Provider counters and underlying settled operations are reported separately from model-issued calls. Timing includes the response-buffering recorder, process startup and tool execution. The experiment supplies no time-to-first-token or streaming latency measure. Claude's fallback prices are excluded.

All 12 cold starts had zero cached input on their first generation request. Token and model-issued call counters cover all 367 generation requests. No separate token-count request was observed. Every traced IPv4 connection targeted loopback and no IPv6 connection was observed. LocalLM advertised 36 tools and kept a byte-stable leading system message and sorted tool definitions in all 12 runs. Its first compact tool schema JSON occupied 23,948 bytes. Claude bare mode advertised two tools, `Read` and `Edit`, despite the five-name allowlist. The actual tool surfaces, templates and approval policies differed.

| Metric across 12 runs per harness | LocalLM | Claude Code |
| --- | ---: | ---: |
| Strict passes | 0 | 0 |
| Factual field matches | 2 | 0 |
| Strict-format matches | 0 | 0 |
| Harness delivered a completed turn | 12 | 7 |
| Runs with a known permission denial | 10 | 0 observed, 1 unknown |
| Turn caps | 0 | 4 |
| Process timeouts | 0 | 1 |
| Runs preserving fixture files | 12 | 12 |
| Generation requests | 34 | 333 |
| Model-issued tool calls | 40 | 1,286 |
| Recognized tool calls | 40 | ≥1,285 |
| Settled underlying tool operations | 30 | ≥1,282 |
| Successful tool operations | 30 | ≥1,102 |
| First-request input tokens | 7,059–7,080 | 764–786 |
| Total input tokens | 252,950 | 2,060,401 |
| Cached input tokens | 204,047 (80.67%) | 2,008,868 (97.50%) |
| Uncached input tokens | 48,903 | 51,533 |
| Requests with positive cache | 28 of 34 | 327 of 333 |
| Output tokens | 2,147 | 31,377 |
| Median wall time | 7.00 s | 4.40 s |
| Maximum wall time | 31.33 s | 240.01 s |

| Harness and cache state, six runs each | Input | Cached | Uncached | Cached share | Median wall time |
| --- | ---: | ---: | ---: | ---: | ---: |
| LocalLM cold | 126,475 | 80,818 | 45,657 | 63.90% | 10.34 s |
| LocalLM warm | 126,475 | 123,229 | 3,246 | 97.43% | 4.67 s |
| Claude Code cold | 794,940 | 771,771 | 23,169 | 97.09% | 4.40 s |
| Claude Code warm | 1,265,461 | 1,237,097 | 28,364 | 97.76% | 56.79 s |

LocalLM supplied the right `code` with an extra field twice, and often attempted an advertised tool denied by the configured rules. Claude returned a line number in `code` on the single-file tasks, repeatedly read files on the four-file tasks, and timed out on one warm join after 62 model requests. The trace correction remeasured every Claude run and preserved the original prompts, grades, timing and provider counters. It adds only observed tool counts to the timed-out trial. Counts for that trial are lower bounds.

LocalLM made fewer generation requests and settled fewer operations on these failed tasks. There is no call-count or latency comparison per successful task. Its initial prompts were larger. Claude Code's higher cached fraction coincided with more aggregate uncached input and less cold uncached input. Warm uncached input was lower in LocalLM. Repeated work and the recorder's buffering limit the interpretation of cache and wall time. Neither harness met the fixed task contract with this model and setup. [All 24 runs and effective commands](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/comparison.json) · [Fixtures and golden answers](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/comparison-fixtures.json) · [Prefix, schema and connection checks](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/comparison-prefix.json) · [Runtime configuration](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/comparison-runtime.json) · [Timed-out trace counters](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/comparison-timeout.json)

## Dependency verification

`npm audit --json` reported zero vulnerabilities. The retained ZIP and Tokio upgrades remove the earlier ZIP and Tokio advisories from the current scan. GLib 0.18.5 keeps the upstream version and ABI, with a vendored upstream pointer-mutability backport. The source integrity verifier and optimized behavior test passed. The earlier unpatched control also passed, so the evidence for the original unsoundness is advisory and source analysis. [GLib advisory](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) · [Backport provenance](/home/papan/Desktop/Projects/localLM/src-tauri/vendor/glib/LOCALLM-BACKPORT.json)

Cargo Audit 0.22.2 scanned a fresh RustSec database checkout at `7a40350e1b8ba5b656d909f88222a8059a83d88d`. It reports **one unresolved advisory and 13 unmaintained dependency warnings**. `RUSTSEC-2023-0071` is present via `sqlx-mysql 0.8.6 → rsa 0.9.10`. In that authentication path the client parses the server's public key and encrypts the nonce-masked password with RSA-OAEP. The timing advisory concerns private-key operations. This is a scoped applicability assessment for that call path. [RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2023-0071.html) · [Audit JSON](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/rust-audit.json) · [Dependency tree](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/rsa-tree.txt) · [npm audit JSON](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/npm-audit.json)

## Final gates

| Command or fixture | Result |
| --- | --- |
| `cd src-tauri && cargo test --lib` | 1,107 passed, 0 failed, 37 ignored |
| `cd src-tauri && cargo check --lib` | Passed |
| `npx vitest run` | 909 passed, 1 skipped across 122 test files |
| `npm run typecheck` | Passed |
| `npm run build` | Passed with the existing large-chunk warning |
| `npm audit --json` | 0 vulnerabilities |
| `python3 scripts/verify-glib-backport.py` | Passed |
| Optimized vendored GLib test | 1 passed |
| Selected ignored HTTP, approval, child and Node inspector fixtures | 21 passed |
| Actual Rust and TypeScript LSP, semantic impact, debugpy and CodeLLDB | 6 passed |
| PostgreSQL and MySQL | 2 passed on the finished binary |
| Live Hugging Face catalog and verified config download, Linux runtime flags | 2 passed. Catalog returned 30 search hits, 28 GGUF entries, and a 5,232-byte verified download. |
| Extra Node checks for transport, render, storage, search and weather | 29 passed |
| Python latency runner, comparison grader and timeout trace | 10, 10 and 1 passed respectively |
| Opt-in MiniCPM fixture | 1 Vitest test passed with two passes through nine scenarios, including an active cancellation followed by `PONG` each time. The fixture observed zero external requests. |
| Finished-build elapsed fixture | Passed the 8-second smoke and both full durations |
| `git diff --check` and `graphify update .` | Passed |

The opt-in entries overlap the 37 default Rust ignores. The comparison and elapsed repetitions are reported in their own sections. Repeated focused checks of already counted tests are excluded from the default totals. [Commands and fixture setup](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/commands.txt) · [Verification output](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/verification.txt) · [Ignored inventory](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/ignored-inventory.txt)

## Open and blocked coverage

| Item | State | Remaining work |
| --- | --- | --- |
| Browser and packaged UI | Unverified | Actual webview reload/reopen, pending approval and typed MCP form recovery, Settings, artifact links, terminal and process controls, and graceful desktop shutdown. Backend and React coverage is listed above. |
| Windows and macOS | Unverified | Native process ownership, packaged UI, installers and platform-specific archives. The Windows CUDA archive case requires the pinned archive inputs. |
| Native Linux perf | Blocked | The perf event probe failed with `perf_event_paranoid=4`. Linux native capture remains untested on a host with access. |
| Isolated X11 driver | Blocked | The fixture's Xvfb dependency is absent. |
| Dependency maintenance | Mitigated and open | GLib uses a source-verified backport. The RSA advisory and 13 unmaintained versions remain in the scan. |
| Durable unattended scheduling and long-term accuracy | Open and unverified | Explicit resume is tested. Autonomous continuation across app restarts, long outages and representative elapsed multi-day task completion need separate implementation and evidence. |
| Comparative accuracy, efficiency and streaming latency | Unverified beyond the recorded scope | Require strict successes across representative coding tasks, matched effective tool surfaces where possible, and streaming measurements without a buffering recorder. |

## Source and resource integrity

The starting inventory had 1,213 hashed files and one separately listed embedded showcase repository. The nine intentional edits cover stream validation, its registered regression fixture, submission IPC and its callers, and the comparison recorder. The other 1,204 inventoried files are byte-identical, including the unrelated dirty work. HEAD and the empty index matched the starting state. New entries consist of this report and selected evidence.

The run used isolated application databases, generated fixture credentials, UUID-named database containers, temporary workspaces, and one owned local model process at a time. The comparison retained failures and effective commands. The prior reports and evidence remain available. [Source integrity](/home/papan/Desktop/Projects/localLM/docs/audits/harness-revalidation-2026-11-09/source-integrity.json)
