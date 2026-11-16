# LocalLM harness acceptance and remediation

9 November 2026 · Linux x86-64 · repository HEAD `609209e7` plus the existing dirty tree

PostgreSQL and MySQL passed real disposable-database acceptance. The registered harness rejected unfinished or contradictory tool streams, recovered a truncated Responses round without dispatching its sibling write, and blocked four exact write repeats across two explicit resumes of an aged checkpoint. Browser and packaged desktop flows remain unverified.

**The comparison used one local MiniCPM5-2B Q4_K_M model for both harnesses.** LocalLM and Claude Code each passed **0 of 12** strict runs. The experiment measures this model and fixture configuration. Production Claude models and general coding-task accuracy were outside its scope.

## Findings and closure

P1 covers execution ownership, side effects, or loss of task constraints. P2 covers protocol recovery, correctness, reporting, or efficiency. The earlier audit and repair report contain the original reproductions and code changes. This run reviewed the interactions and reran the applicable gates. [Original audit](/home/papan/Desktop/Projects/localLM/docs/HARNESS_RELIABILITY_AUDIT_2026-11-08.md) · [Earlier repair report](/home/papan/Desktop/Projects/localLM/docs/HARNESS_RELIABILITY_FIX_RESULTS_2026-11-08.md)

| Finding | Severity and state | Reproduction, cause, and evidence |
| --- | --- | --- |
| H01 · Child lifecycle and interrupted work | P1 · Fixed in scoped tests | A dropped or failed child could retain its registration and lose settlement. Lease-owned admission, durable dispatch and restart reconciliation now retain uncertainty and release capacity. Foreground abort, detached interruption, startup reconciliation and initialization-failure tests passed. |
| H02 · Descendants outlive their owner | P1 · Fixed on Linux fixture paths | Killing the owner previously left ordinary descendants running. An owned process group and guardian now terminate the group. Process exit, app-owner death and registry-drop tests passed. See C09 for the stop-response race found in this run. |
| H03 · Multipart instructions disappear in compaction | P1 · Fixed | The original handoff read only string content. It now retains text constraints, attachment references and the archive. C06 repairs the reference coordinates when archiving a live exchange. |
| H04 · Missing child project rules | P1 · Fixed | Foreground and background children omitted the applicable workspace instruction chain. Captured child requests now include the root-rule sentinel and selected skill context. |
| H05 · Unfinished turns presented as complete | P1 · Fixed in the covered cases | The 64-request budget fixture retains stable schemas, ends with `tool_choice=none`, and reports partial. Pending graph tasks and unknown ledger operations also appear in the partial outcome. |
| H06 · Completed work reappears as paused | P2 · Fixed | Research ledgers once stayed running after a completed answer. Run binding and settlement preserve completion on reopen, reconcile 55 older completed ledgers, and reuse the task ID when resuming. C08 covers exact replay guards across segments. |
| H07 · Reasoning-only child accepted | P2 · Fixed | A valid terminal frame with only reasoning used to produce an empty success. The child now fails and retains provider-reported usage. |
| H08 · Unlimited background admission | P1 · Fixed in one application process | The parent limit did not bound background fan-out. The shared per-database limit is eight children in the process. The ninth is refused and a released slot can be reused. |
| H09 · Read-only program calls invalidate verification | P2 · Fixed | The wrapper previously marked all work as mutating. Typed nested effects preserve verification for pure reads and stay conservative for hooks, writes, and opaque work. A scripted pair now takes 2 model requests and 1 outer call when batched versus 3 and 2 directly, with 2 real reads and the same answer in both paths. |
| H10 · Missing delegated usage | P2 · Fixed | Child requests previously dropped usage and cache counters. Per-round observations, settlement and parent diagnostics now preserve reported values and leave absent measurements unknown. |
| H11 · Quoted handoff text overrides saved context | P2 · Fixed | A literal display marker used to be interpreted as saved metadata. Internal fields now carry the handoff and stay off provider payloads. C07 also closes a separate legacy checkpoint-prefix case. |
| H12 · npm advisory updates | P2 · Fixed at scan time | Updated lockfile and Zod patch level. The final npm scan reported zero vulnerabilities across 430 dependency entries. |
| Concurrent SQLite writes and credential rotation | P2 · Fixed in fixture paths | Writer reservation and atomic event allocation preserve 160 messages and 160 run events under concurrent writes. A scripted turn uses the rotated credential on its next model request, rejects an account change, and redacts both credential values from errors. |

| Finding in continuation | Severity and state | Reproduction, cause, and repair evidence |
| --- | --- | --- |
| C01 · MySQL 8.4 metadata decoding | P2 · Fixed | Binary encodings and uppercase metadata names obscured column, constraint and ownership-marker text. Decode UTF-8 only at known metadata boundaries and normalize names for named lookups. The real test sees `positive_id`, its enforced CHECK and the new column. A user query selecting a binary value still reports explicit omission. |
| C02 · Fresh MySQL role authentication and client options | P1 · Fixed for the tested client | The first native connection for a fresh `caching_sha2_password` role needs TLS or RSA password exchange. Explicit `disable` adds `--get-server-public-key`. Native clients use the profile’s TCP endpoint and ignore machine option files and login paths. The runner proves its disposable MySQL client can see an invalid default option, then the rehearsal succeeds with that option file present. |
| C03 · MySQL QUICK check and cleanup confirmation | P1 · Fixed | MySQL 8.4.13 rejects `PREPARE ... CHECK TABLE ... QUICK` with error 1295 and accepts the same statement via the text protocol. SQLx now uses that path for the quoted scratch identifier. Error rows fail the rehearsal, and coverage requires a positive status for each checked table. Cleanup rechecks both resource names for absence before certifying success. |
| C04 · Unclosed Claude tool block and late compatible deltas | P1 · Fixed | An assembled JSON object previously reached dispatch without `content_block_stop`, and a later delta or conflicting finish reason could change a compatible stream after its declared finish. The shared stream path enforces the native tool-block stop, protocol-specific terminal event, and consistent finish reason. The registered Claude fixture leaves the target file absent and saves a failed assistant with zero tool audits. |
| C05 · Refusal dropped or combined with execution | P1 · Fixed | Compatible `delta.refusal` was ignored as empty visible output. Native Responses could treat a refusal with a sibling function as a tool round. The shared stream exposes refusal text, disallows a refusal plus tool calls, and keeps usage-only tails. The registered compatible turn persists the refusal with one request. |
| C06 · Active-compaction attachment offsets and settlement | P1 · Fixed | The active handoff indexed a temporary message array whose first element was missing from the saved artifact. The first active archive now contains the actual request at that index. Repeated compaction preserves that reference without another attachment copy, keeps filename extensions in constraints, and archives only prefixes with exactly one result per call ID. The tight-window parallel regression keeps the missing-result exchange intact. |
| C07 · Legacy checkpoint text overwrites a real user turn | P1 · Fixed | Reopening a cached handoff could replace a later literal user message beginning with the old checkpoint notice. The synthetic placeholder now carries continuation metadata, and restoration targets only that placeholder. A reopen and recompact fixture retains the literal request verbatim. |
| C08 · Exact side effects can be reissued after explicit resume | P1 · Fixed for linked audited parent and program calls | The dispatch guard started empty on resume, and staging consumed its prompt checkpoint before context preflight succeeded. The ledger now retains run membership, seeds the existing shared guard from prior durable audits, and clears the staged note only on adoption. Startup marks a run interrupted during tools outcome-unknown. A three-day timestamp fixture really reopens SQLite, performs two registered resumes, denies four attempted exact writes, retains the completed file and keeps the uncertain operation partial. Multipart resume retains the original parts and adds the saved note. |
| C09 · Stop acknowledges a running snapshot | P2 · Fixed | A full suite run reproduced `process_stop` returning `running` after successful tree termination. The supervisor now publishes terminal state and bounded drained output before acknowledging the explicit stop. Verification sealing can continue afterwards. The acknowledgement regression and final full suite passed. |
| C10 · Truncated Responses function batches skipped bounded recovery | P2 · Fixed | Valid draft events followed by `response.incomplete` with `max_output_tokens` were rejected while reading the incomplete function item. Incomplete items remain drafts and reach the existing output-limit retry. The registered regression drops the entire truncated batch, including a complete sibling write, then performs one real read on the follow-up. [Official OpenAI streaming events](https://developers.openai.com/api/reference/resources/responses/streaming-events) |
| C11 · Pause before dispatch creates false uncertainty | P2 · Fixed | Pausing a not-yet-dispatched call created a running ledger operation. Recovery promoted it to unknown even though dispatch had never started. The boundary now keeps named pending work and settlement preserves it. The regression settles, recovers and checks the paused ledger with no uncertain operation. |

The continuation also retains the ZIP and Tokio upgrades, fallible ZIP filename handling, the GLib backport, and LSP readiness and bounded `ContentModified` retry for unchanged pure reads. The Rust and TypeScript navigation and semantic-impact acceptance tests passed again.

## Database acceptance

Both engines ran from digest-pinned local images, with a fresh UUID-named fixture on a loopback-only port. The runner verified container absence with a successful Docker query. PostgreSQL covered read-only access, disposable rehearsal, source preservation and cleanup. MySQL used a separate SELECT-only source role, a generated scratch role scoped to its database, and the real native dump and restore clients.

On MySQL, the successful migration returned an enforced CHECK, a primary key, the new column and `CHECK TABLE QUICK` status `OK`. The failing migration reached the CHECK violation after scratch DDL, returned an error and cleaned up. Both observations retained the original source row and schema. The test queried the server for absence of each generated role and schema and verified that repeated cleanup was safe. The first run in this continuation reproduced the QUICK-check failure after successful dump and restore. [Database evidence](/home/papan/Desktop/Projects/localLM/docs/audits/harness-acceptance-2026-11-09/databases.json) · [MySQL fixture output](/home/papan/Desktop/Projects/localLM/docs/audits/harness-acceptance-2026-11-09/mysql-acceptance.txt)

The TLS mapping remains `DISABLED`, `REQUIRED`, or `VERIFY_IDENTITY` according to the profile. RSA public-key retrieval is confined to explicit TLS disable. The real fixture exercised that mode. The required and identity-verifying modes were checked in source. The native clients document RSA key retrieval for the caching SHA-2 exchange and the login-path environment override for older clients. [MySQL client options](https://dev.mysql.com/doc/refman/8.4/en/mysql-command-options.html#option_mysql_get-server-public-key) · [mysqldump options](https://dev.mysql.com/doc/refman/8.4/en/mysqldump.html#option_mysqldump_get-server-public-key) · [MySQL 8.0 option files](https://dev.mysql.com/doc/refman/8.0/en/option-files.html)

## Protocol and application paths

| Boundary | Evidence on the host |
| --- | --- |
| Compatible Chat Completions | Registered turns exercised fragmented arguments, incomplete and empty replies, refusal and persisted usage, malformed calls, real file writes after bounded retry, and cancellation during argument streaming. Bytewise tests cover fragmented UTF-8 and JSON framing, CR/LF/CRLF, BOMs, comments, multiline data, usage tails and invalid or oversized events. |
| Responses native context | The loopback parent executed a real tool and replayed the original call ID and opaque reasoning. Missing results carry uncertainty. The output-limit fixture suppresses all writes in a truncated round and successfully resumes through the native protocol. |
| Claude native context | Native block indices, argument fragments, thinking signatures, redacted blocks and batched tool results have positive parser and child-path coverage. A registered parent rejected an unclosed write. Subscription refresh and remote authentication used fixture credentials and loopback servers. |
| Local text calls and program calls | The local-template fallback validates complete bare tool markup and keeps Markdown examples inert. Programmatic and mixed program/model groups preserve the canonical audits. The registered PTC comparison checks real underlying file reads, stable results and no spurious verification. |
| Retry and dispatch | Pre-stream retry tests cover rate limits, `Retry-After`, bounded attempts, credential redaction and cancellation during backoff, headers, and the error body. Disconnects and failures after a partial successful stream do not replay that stream. Authorization is checked at dispatch, including grouped reads, nested calls, approval changes and later paths in multi-target edits. |
| Journal and UI contracts | The invoke-name registration check, typed tool output contracts, artifact bounding with lifecycle handles, live/replayed reasoning and approvals, and reattaching to a pending approval all passed. MCP typed input and detached authority are exercised at the backend and dialog-test boundaries. React tests exercise the Settings-only presentation flow, rendering, response metrics and lifecycle controls. |
| Linux process lifecycle | Foreground and background ownership, queued turns, dropped IPC, owner exit, process-group termination, and bounded output finalization were exercised. The profile fixture used a real Node inspector. |

## Elapsed endurance and restart

<!-- ELAPSED_RESULTS -->

Each full run used 24 real file reads, 25 model requests, a protected file, SQLite, and scripted SSE with heartbeats at most 30 seconds apart while waiting for each response. One-hour and two-hour runs used different paces and isolated workspaces. Completion required correct read evidence, no live registered turn, durable compaction artifacts and canonical history after reopening, with a completed ledger and non-partial terminal turn. The updated fixture also checks the durable run row and all archived artifacts. RSS values are sampled process memory on a shared host.

The previous session’s run stopped near 2,652 seconds and supplied no one-hour completion. This continuation also completed a separate one-hour and two-hour run on the starting binary. Those starting workspaces were under the repository and could inherit its instruction chain. The later fixture workspaces were outside it.

The three-day restart test ages timestamps and uses a real SQLite reopen. Its first durable segment contains one completed write and an unacknowledged write audit. Two explicit resumes retain one ledger, block both exact writes in each new segment, allow read-only inspection, and report unresolved work. The tests also cover preflight-safe staging and model-round recovery from saved tool results with tools disabled. This is accelerated age coverage.

| Capability | Evidence and limit |
| --- | --- |
| Work while the app stays running | One registered read-only workload completed at each real elapsed duration. A parent turn caps at 64 model rounds. Child turns default to 4 rounds and cap at 8. Individual managed shell runs default to 60 seconds and cap at 300. The model HTTP timeout is 600 seconds and the stream idle guard is 300. |
| Resume after interruption | Explicit checkpoint resume is verified across reopen and multiple segments. The guard covers exact signatures on linked audited calls. It cannot prove semantic equivalence between altered commands or recover membership never recorded by older builds. |
| Continue across restart without input | Open. Startup reconciles active work and permits explicit resume. No durable autonomous supervisor was exercised. |
| Finish a representative multi-day task accurately | Unverified. Remaining evidence would require real multi-day workload completion with measured resource use, outages and restart fault injection, plus outcome checks independent of the model. |

## Claude Code comparison

Claude Code 2.1.319 ran in bare, noninteractive, no-session-persistence mode. Both harnesses used llama.cpp b10855 commit `7d701b592` on Vulkan1, the RTX 2050, with one model slot, a 32,768-token context, 1,024 generated tokens per request, no reasoning, temperature 0, top-p 1, and seed 0. The model was `openbmb/MiniCPM5-2B-GGUF` revision `2079a22f3beaa4e306449978533478fe0522f4b3`, GGUF SHA-256 `ec2d5801640099e97d8d7e8003ad4d81f336e757811f03a26173dddf386602fd`. The recorder forwarded Claude’s native Messages requests and LocalLM’s Chat Completions requests unchanged. The experiment used no protocol translation adapter.

The suite used two fixed tasks, three input seeds (`20261108` through `20261110`), and one cold and immediate warm run per task, seed and harness. It alternated harness order, restored byte-identical inputs, kept the golden JSON outside the workspace, and checked file contents and filesystem metadata for mutation. A slot erase preceded each cold run with the separate RAM prompt cache disabled. All 12 first cold requests reported zero cached tokens. Each harness had a 64-turn bound and a 240-second process deadline. LocalLM also reserves a final model round and applies its own repetition and permission policy.

The chosen permission mapping allowed corresponding read, glob, search, create and edit capabilities. LocalLM used the Chat preset, Workspace source, text presentation in Settings and five matching permission rules. Its production schema still advertised 35 tools, including tools denied by those rules. The leading system message and sorted tool definitions stayed identical across rounds in all 12 LocalLM runs. The CLI was configured for `Read,Glob,Grep,Write,Edit`, and its actual bare-mode requests exposed `Read` and `Edit`. Those surfaces, prompt templates, permission behavior and endpoint formatting are observable differences. Every traced IPv4 connect attempt targeted loopback, with zero IPv6 attempts. The two earlier experiments are retained in local QA. Their workspace inheritance or turn and permission mismatches exclude them from the result below. [Prefix and schema checks](/home/papan/Desktop/Projects/localLM/docs/audits/harness-acceptance-2026-11-09/comparison-prefix.json)

The grader ran ten fixed positive and negative cases first. Strict format requires a single bare JSON object with exactly the requested keys and types, with duplicate keys rejected. The separate factual score checks requested field values in a parseable object and allows extra keys or a fenced object. Success also requires terminal completion, successful tool evidence, unchanged inputs and no permission denial. Failed and capped runs contribute to the request, operation, token and latency totals. CLI fallback dollar estimates were excluded. [All 24 rows and commands](/home/papan/Desktop/Projects/localLM/docs/audits/harness-acceptance-2026-11-09/comparison.json) · [Fixtures and expected answers](/home/papan/Desktop/Projects/localLM/docs/audits/harness-acceptance-2026-11-09/comparison-fixtures.json)

| Metric across all 12 runs per harness | LocalLM | Claude Code |
| --- | ---: | ---: |
| Strict passes | 0 | 0 |
| Factual field matches | 2 | 0 |
| Strict-format matches | 0 | 0 |
| Harness delivered a completed turn | 12 | 6 |
| Runs with permission denials | 10 | 0 |
| Runs reaching the turn cap | 0 | 6 |
| Runs leaving fixture files unchanged | 12 | 12 |
| Model requests | 34 | 396 |
| Model-issued tool calls | 40 | 1,422 |
| Settled underlying tool operations | 30 | 1,422 |
| Successful underlying operations | 30 | 1,358 |
| First-request input token range | 7,059–7,080 | 765–787 |
| Input tokens | 252,950 | 2,073,796 |
| Cached input tokens | 204,047 | 2,032,636 |
| Uncached input tokens | 48,903 | 41,160 |
| Output tokens | 2,147 | 27,432 |
| Median wall time | 6.56 s | 31.67 s |

| Harness and cache state, six runs each | Input tokens | Cached | Uncached | Cached share | Median wall time |
| --- | ---: | ---: | ---: | ---: | ---: |
| LocalLM cold | 126,475 | 80,818 | 45,657 | 63.90% | 9.64 s |
| LocalLM warm | 126,475 | 123,229 | 3,246 | 97.43% | 4.55 s |
| Claude Code cold | 1,089,713 | 1,065,987 | 23,726 | 97.82% | 53.66 s |
| Claude Code warm | 984,083 | 966,649 | 17,434 | 98.23% | 31.17 s |

LocalLM had fewer requests and tool operations on these failed tasks. The comparison supplies no per-success efficiency result. Claude Code hit the cap in all six four-file joins and repeatedly reread files. In the single-file task, it returned a line number in the requested `code` field. LocalLM returned the right `code` with an extra field twice and often tried an advertised but denied tool. LocalLM’s first requests were larger. Claude Code had the higher aggregate cached share and lower total uncached input, while LocalLM had lower warm uncached input. Both the numerator and denominator include repeated failed work. The recorder buffers responses, so wall times include buffering, tool work and client overhead. Time to first token and streaming latency are unverified.

## Security and final checks

ZIP 9.4.2 and Tokio 1.54.1 remove the advisories reported for the starting dependency versions. The GLib 0.18.5 backport retains the crate version and verifies upstream source hashes with one patched source file. Its optimized behavior test passed. The unpatched control in the previous session also passed, so the original undefined behavior was established by source and advisory review. [GLib advisory](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) · [Backport provenance](/home/papan/Desktop/Projects/localLM/src-tauri/vendor/glib/LOCALLM-BACKPORT.json)

Cargo Audit 0.22.2 reports **one unresolved advisory and 13 unmaintained dependency warnings**. `RUSTSEC-2023-0071` remains on `rsa 0.9.10` through `sqlx-mysql 0.8.6`. The reviewed authentication call parses the server’s public key and encrypts the password with RSA-OAEP. The advisory concerns timing leakage during private-key operations. This scoped call-path assessment leaves the dependency advisory visible. [RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2023-0071.html) · [Rust audit JSON](/home/papan/Desktop/Projects/localLM/docs/audits/harness-acceptance-2026-11-09/rust-audit.json)

| Check | Result |
| --- | --- |
| `cd src-tauri && cargo test --lib` | 1,103 passed, 0 failed, 37 ignored |
| `cd src-tauri && cargo check --lib` | Passed |
| `npx vitest run` | 909 passed, 1 skipped in the default run |
| `npm run typecheck` | Passed |
| `npm run build` | Passed with the existing large-chunk warning |
| `npm audit --json` | 0 vulnerabilities reported |
| `python3 scripts/verify-glib-backport.py` | Passed |
| Optimized GLib backport behavior test | Passed |
| Selected normally ignored Rust loopback and Node profiler fixtures | 21 passed |
| Real LSP navigation, semantic impact, debugpy and CodeLLDB | 6 passed |
| Real PostgreSQL and MySQL acceptance | 2 passed |
| Real public model catalog and verified download, Linux runtime flags | 2 passed |
| Opt-in MiniCPM fixture and direct local-runtime cancellation probe | Frontend fixture passed, including two passes through nine scenarios. The direct probe closed an active stream and received `PONG` on the next request. |
| Opt-in LocalLM real-model comparison and endurance entries | Both executed separately. Repetitions are included in the comparison and elapsed tables, with benchmark failures retained. |
| Node transport, rendering, storage, search and weather tests | 29 passed |
| Python latency-runner tests and comparison grader cases | 10 passed each |
| `git diff --check` and `graphify update .` | Passed |

The 37 default Rust ignores are a separate inventory. The entries executed above overlap that inventory and are excluded from the default pass count. The remaining entries are the real browser fixture, isolated X11 driver fixture, native profiler and Windows CUDA archive extractor. The runtime-flags probe used Linux llama.cpp. The frontend’s opt-in live MiniCPM fixture ran separately from the default Vitest count. [Final validation and ignored inventory](/home/papan/Desktop/Projects/localLM/docs/audits/harness-acceptance-2026-11-09/verification.txt) · [Live model fixture](/home/papan/Desktop/Projects/localLM/docs/audits/harness-acceptance-2026-11-09/live-model-fixtures.txt)

## Open and blocked coverage

| Item | State | Remaining evidence |
| --- | --- | --- |
| Browser and packaged native UI | Unverified | Conversation reopen, approvals and MCP forms across a real webview reload, Settings, artifact links, transcript and process controls, and full graceful desktop exit. The browser preview constraint was honored. |
| Native Windows/macOS and installer assets | Unverified | Platform-owned acceptance for process handling, native UI and installation. The cached pinned Windows CUDA archives needed by the extractor test were absent. |
| Native Linux profiler | Blocked | `perf record` could not open an event with `perf_event_paranoid=4`. The host setting was preserved. |
| Isolated Linux X11 input | Blocked | Xvfb was absent. The fixture requires its own virtual display. |
| Dependency maintenance | Mitigated and open | GLib uses a verified compatibility backport. The RSA advisory and 13 unmaintained Rust dependencies remain visible. |
| Long-running coordination | Open | Current tests establish bounded registered turns and explicit restart/resume. Unattended cross-restart coordination, recovery from extended outages and completion of real multi-day work still need implementation or evidence. |
| Model and tool efficiency | Unverified beyond the fixtures | The same-model pilot had zero strict successes. Successful task counts on representative workloads are needed before comparing accuracy, calls per success, or inference latency. |

## Source and resource integrity

The initial SHA-256 inventory covered 1,195 tracked and nonignored entries. The final comparison records each changed pre-existing file and its before/after hash. Existing dirty changes outside the listed remediation files were preserved. The comparison and model runtime were stopped and reaped, and database containers were removed and checked by name. Selected evidence contains fixture outputs and scan summaries. The local QA directory retains the full logs and build binaries. [Source-integrity manifest](/home/papan/Desktop/Projects/localLM/docs/audits/harness-acceptance-2026-11-09/source-integrity.json) · [Reproduction commands](/home/papan/Desktop/Projects/localLM/docs/audits/harness-acceptance-2026-11-09/commands.txt)
