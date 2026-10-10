# Backend audit — 7–8 October 2026

The backend was checked against the actual Rust modules, Tauri command registration, frontend call sites, and executable tests. Graphify output was treated as an index, not evidence that a feature exists or works. The entry workspace already contained substantial uncommitted work; that work was preserved. This report describes changes made during this audit separately from existing behavior.

There are **70 Rust source files**: 68 modules declared by `lib.rs`, plus `lib.rs` and `main.rs`. The initial 69-file count omitted `system_tools.rs`; the inventory below includes it. `browser_agent.js` is an additional in-page driver. Coverage here means source inspection and the stated tests, not certification of every live integration or a claim that every feature works.

## Changes and evidence

| Change | Actual source | Why / regression evidence |
| --- | --- | --- |
| Query only the research context that is used | `store.rs`, `web_search.rs` | Replaced two full-history loads with an indexed scan of role/content for the latest six nonblank messages and a direct latest-user query. Preserves chronological order, Rust whitespace handling, Unicode character limits, and the latest blank user message's original semantics. Equivalence fixture includes 1,003 messages and large reasoning/tool audits. |
| Reduce startup repair allocations | `store.rs` | Recovery now selects only lifecycle event kinds and loads payloads only for tool calls. Recovery still closes incomplete tails, pairs unanswered calls, and does not replay writes. Existing restart/atomicity tests pass. |
| Keep workspace reads off the database/UI path | `workspace_ui.rs` | Bind the project under a short database lock, then perform file inspection/search with `spawn_blocking`. Tests verify project binding, rejection of path escape, and release of the database before filesystem work. Project membership checks use a `HashSet` instead of scanning all projects for every chat. |
| Bound Git capture and parallelize independent reads | `workspace_ui.rs` | Capture at most 400,000 stdout bytes / 8,000 stderr bytes, drain remaining output, retain the existing 100,000 / 2,000 character prefixes, timeout, hidden Windows process, and diagnostics. Diff, branch, branch list and status run concurrently after any validated branch switch. A real Git fixture verifies branch/status, exact Unicode diff truncation and errors; a small pipe fixture verifies draining oversized output without deadlock. |
| Preserve terminal Unicode and bound replay efficiently | `terminal_ui.rs` | The previous per-read UTF-8 decoding corrupted characters split between PTY reads. Retain incomplete suffixes, flush malformed bytes safely, use a `VecDeque` for capped scrollback, and serialize recording/emission with attachment replay. Regression covers every Unicode read boundary, byte cap boundaries, and actual PowerShell PTY shutdown. |
| Gate browser-driven navigation | `inspector_browser.rs` | Apply the existing HTTP/HTTPS/about:blank URL validation to in-page navigation and popups as well as typed/agent URLs. HTTPS redirects remain allowed. Tests reject executable/local/custom schemes. A full native WebView2 browsing session remains outside the executed coverage below. |
| Remove unused installer fixture scaffolding | `runtime_install.rs` | Removed unused test server/stub code responsible for seven baseline warnings. Added a small cached, checksum-verified ZIP installation test covering publication, cache removal and reuse without a network request. This is test code cleanup, not a production-code reduction. |
| Simplify startup recovery dispatch | `lib.rs` | Replaced two one-element loops with direct runtime/model recovery branches; keeps cancellation and recovery behavior. |
| Correct the system clock tool | `system_tools.rs`, `Cargo.toml`, `Cargo.lock` | Reproduced a panic for timezone `+😀` caused by byte slicing inside UTF-8. Numeric offsets now reject non-ASCII input. Replaced fixed winter offsets for IANA regions with date-specific IANA rules, preserving fixed legacy abbreviations/numeric offsets and case-insensitive names. Explicit January/July tests cover New York, London, Sydney and Kathmandu; the report ISO offset and epoch describe the same instant. |

The timezone change uses pinned `chrono-tz = "=0.10.4"`; the lockfile adds only `chrono-tz`, `phf` and `phf_shared`. The [official chrono-tz documentation](https://docs.rs/crate/chrono-tz/0.10.4) documents the IANA implementation, compatibility with Chrono 0.4, and conversion APIs. Keeping a hand-written regional offset table would continue returning wrong times during daylight saving. The bundled timezone database requires dependency updates when regional rules change.

## Verification

Run from `src-tauri`:

```text
cargo test
cargo clippy --all-targets -- -D warnings
cargo test --lib -- --ignored
cargo test --lib research_window_matches_history -- --nocapture
```

Final results after the timezone fix:

| Check | Result |
| --- | --- |
| `cargo test` | **449 passed, 0 failed, 2 ignored**; 62.63 s test execution. Binary test target and doc-test target compile and pass, each with zero registered tests. |
| `cargo clippy --all-targets -- -D warnings` | **Passed** with no warnings; 38.85 s. |
| Explicit ignored native/cache checks | **2 passed, 0 failed**, 64.85 s; executed earlier in this audit. |
| Final source/test inventory | **70 Rust files, 451 registered tests**; the table below sums to those counts. |

The two environment-dependent ignored tests probe current native llama-server flags and extraction/hash/manifest checks against the two cached pinned CUDA archives. The final changes to Git/timezone handling do not touch those tested surfaces. No runtime archives were downloaded to run those checks.

Benchmark output, from the equivalence test in a debug build:

```text
Research history (1,003 messages, debug build): median 20.8803ms full reads -> 484.4µs projected reads
```

Five iterations measure two former full `messages()` reads against the two projected queries. The result is about **43× faster context retrieval** for this fixture, with identical output. A concurrent full-suite run measured `36.0888ms -> 853.5µs` (about 42×); use the isolated result for comparison. These are local SQLite/context-preparation timings, not token generation speed or an end-to-end agent benchmark.

Code size is compared with the saved current-workspace entry baseline, not Git HEAD, because HEAD omits the user's existing work. Across the eight edited Rust files, production sections change by **+57 lines**, test sections by **+78 lines**, and the files by **+135 lines total**. Sections are split at the final `#[cfg(test)]` test module; unchanged inline test helpers remain in the production-section accounting. Production reductions are `web_search.rs` −7, `lib.rs` −21 and `system_tools.rs` −1; bounded process/terminal handling and projected queries add necessary logic. The removed installer scaffolding is counted in test code. This backend pass improves resource use and correctness; the comprehensive audit records the separate frontend/source reductions. No framework or generic orchestration layer was added.

## Source and feature coverage

Counts are registered direct Rust tests for the final source, including the two explicitly executed ignored tests. A zero denotes no direct tests in that file, not an absent feature. Command wiring and DTOs are also exercised from other modules. The inventory is exhaustive by source file; test counts are not a line-coverage metric.

| File in `src-tauri/src` | Feature / source review focus | Tests |
| --- | --- | ---: |
| `lib.rs` | App state, plugin setup, startup recovery, registered IPC handlers | 0 |
| `main.rs` | Desktop entry point | 0 |
| `commands.rs` | Bootstrap, model selection/load rollback, readable runtime snapshot | 5 |
| `attachments.rs` | Attachment decoding, model content and image URL policy | 1 |
| `chat.rs` | Turn assembly, streaming, tools, cancellation, planning/research finalization | 49 |
| `agent_run.rs` | Run states, checkpoint/resume, ownership and packaged execution | 8 |
| `arex.rs` | Research controls, evidence, budgets and result synthesis | 7 |
| `continuation.rs` | Bounded correction/retry decisions and unresolved outcomes | 10 |
| `harness.rs` | Offered tool registry, dispatch, search outcomes and argument validation | 10 |
| `tool_calls.rs` | Structured/text tool call parsing, interleaving and incomplete arguments | 8 |
| `tool_compat.rs` | Runtime/model compatibility identity and persisted support | 2 |
| `tool_envelope.rs` | Nested tool schema validation | 1 |
| `prompt.rs` | Frozen prompt prefix, volatile injections and multimodal handling | 8 |
| `context.rs` | Token preflight, response reserve, overflow and runtime errors | 5 |
| `sse.rs` | UTF-8 event framing, CRLF, bounds and truncated streams | 3 |
| `approval.rs` | Single-use, request-bound approvals and user choice | 2 |
| `permissions.rs` | Ask/read/full-access scope and durable defaults | 2 |
| `capabilities.rs` | Catalog parsing, activation and unknown IDs | 3 |
| `presets.rs` | Product tool selections, schema validation and SDK aliases | 5 |
| `store.rs` | SQLite persistence, atomic turns/forks, recovery, settings and research projection | 37 |
| `history.rs` | Replay tool exchanges, denied/interrupted outcomes and legacy audits | 6 |
| `sessions.rs` | Session event/data contracts; exercised by store and history | 0 |
| `compaction.rs` | Tool/turn boundaries, bounded context and persistent checkpoints | 14 |
| `artifacts.rs` | Bounded excerpts, paging and retained large results | 8 |
| `export.rs` | Complete JSON/Markdown and atomic destination replacement | 2 |
| `research_tasks.rs` | Durable task budgets, checkpoint/pause/resume and unknown outcomes | 8 |
| `plans.rs` | Plan parsing, todo validation and current-task focus | 5 |
| `memory.rs` | Local survey, bounded recall and input limits | 3 |
| `inference.rs` | Local/provider wire payloads, typed transport failures and redaction | 13 |
| `providers.rs` | Loopback/remote endpoint validation, model limits and selection | 8 |
| `local_only.rs` | Local inference gate, setup requirements and offline retrieval gate | 3 |
| `claude_adapter.rs` | Legacy Messages-protocol mapping and stream normalization | 2 |
| `subscription_auth.rs` | Legacy token/session parsing, PKCE and credential rotation | 7 |
| `connectors.rs` | MCP lifecycle, discovery, aliases, reconnect, approval and result bounds | 17 |
| `local_mcp_config.rs` | Executable/argument validation and encrypted environment | 1 |
| `local_mcp_process.rs` | Actual stdio handshake, oversized frames, process tree cleanup | 5 |
| `tool_discovery.rs` | Catalog pagination/cursor/name limits | 1 |
| `oauth.rs` | Callback path/origin validation | 1 |
| `vault.rs` | Encrypted credential records, tampering rejection and rotation | 5 |
| `skills.rs` | Pinned package integrity, path/junction controls and activation | 10 |
| `runtime.rs` | Process state, startup limits, model/projector validation and native flags | 5 |
| `runtime_config.rs` | Resource validation, GPU/cache allocation and launch arguments | 11 |
| `runtime_log.rs` | Bounded diagnostics and draining despite write failure | 3 |
| `hardware.rs` | Driver evidence parsing and unavailable readings | 2 |
| `gguf.rs` | Header/metadata parsing and incomplete-file handling | 4 |
| `runtime_archive.rs` | Checksum/manifest extraction and cached pinned real archives | 2 |
| `runtime_install.rs` | Cancellation and cached verified publication/reuse | 2 |
| `runtime_install_commands.rs` | Install IPC coordination; delegates to tested installers/state | 0 |
| `runtime_inventory.rs` | Published/incomplete bundle discovery | 2 |
| `install_recovery.rs` | Staging/cache repair and junction safeguards | 4 |
| `model_catalog.rs` | Built-in catalog identity and path validation | 1 |
| `model_library.rs` | Hub/search shapes, manifest/profile/shard/projector lifecycle | 21 |
| `model_install.rs` | Job identity, cancellation and durable status transitions | 5 |
| `download.rs` | Resume ranges, SHA-256, retries, cancellation and redirect credential policy | 20 |
| `workspace.rs` | Project-bound paths, fresh reads, atomic edits, batch and search bounds | 15 |
| `workspace_ui.rs` | Project/chat metadata, async inspection/search and bounded real Git snapshot | 6 |
| `sandbox.rs` | Local tool policy, fetch SSRF checks, bounds and Docker denial | 14 |
| `execution.rs` | Actual shell/Python/JS execution, streaming, limits and descendant cleanup | 8 |
| `terminal_ui.rs` | Native PTY lifecycle, Unicode decoding and capped replay | 3 |
| `inspector_browser.rs` | Native webview command lifecycle, validated navigation and popups | 1 |
| `browser_agent.rs` | Browser mode/actions, validation and per-run action budget | 2 |
| `web_search.rs` | Worker install, loopback model snapshot, offline mode, deadlines and output bounds | 10 |
| `weather.rs` | Location bounds, forecast response shape and condition labels | 4 |
| `system_tools.rs` | Host clock, IANA/DST conversion, fixed offsets and malformed Unicode | 8 |
| `subagents.rs` | Child task/depth policy, detached support and handoff parsing | 4 |
| `daytona.rs` | Legacy owned sandbox API validation, request bounds and redirect policy | 3 |
| `daytona_cleanup.rs` | Ownership-bound cleanup and ambiguity/cancellation recovery | 2 |
| `daytona_execution.rs` | Product gate refuses new cloud execution before creation | 1 |
| `daytona_journal.rs` | Persistent ownership/operation journal and unknown outcomes | 2 |
| `daytona_settings.rs` | Historical cleanup recovery across credentials | 1 |

The JavaScript browser driver is present and connected through `browser_agent.rs`; its browser DOM behavior is covered in the frontend/browser test work documented in the comprehensive audit. Rust unit tests cover action planning and budgets, not a live WebView2 navigation/redirect or a model-driven interaction.

## Feature decisions and remaining limits

- Keep local inference, project-bound workspace tools, explicit approvals, durable run/task recovery, terminal, browser, research, model/runtime installation, MCP and exports. These implement the product documented in `PRODUCT.md` and have real source/dispatch paths.
- Keep the local-only and deferred Docker/cloud gates. `chat.rs`, subagent assembly and inference tests call the local inference policy; saved unsupported execution settings fail explicitly. Local execution is unsandboxed, as stated in the product. Do not label it sandboxed.
- Legacy remote/subscription adapters and Daytona cleanup/journal code are candidates for a future migration, not blind deletion: old saved selections/audits and owned-resource cleanup depend on them. New cloud execution remains disabled. Removing these safely requires explicit compatibility and data-migration tests.
- **P2 scale:** conversation/index/session lists still materialize entire growing collections. The project lookup is cheaper now, but pagination/virtualization and session-window reads need measurements against thousands of long chats before choosing a larger refactor.
- **P2 setup overhead:** secondary `Store::open` calls repeat schema/seed work for detached tools. A shared-store redesign should preserve the tested rule that secondary connections cannot recover or invalidate an active run.
- **P2 worker overhead:** research actions start a Node worker and verify its installed bytes on each request. Integrity and process isolation are deliberate; profile repeated tool calls before introducing a daemon/cache protocol.
- **Verification semantics:** managed-model inventory `verified` means declared files exist with expected sizes; SHA-256 is checked during download/publication, not on every scan of multi-GB files. A future UI or API must not imply every listing freshly hashes all model bytes.
- **Native/runtime limits:** cached archive extraction, native runtime flag probing, real PowerShell PTY shutdown and local child-process tests ran. The comprehensive audit additionally ran two brief direct-server fixture passes using the cached 2.07 GB MiniCPM GGUF, including model-generated calculation arguments handed through the shipped parser/catalog to workspace execution. Sustained GPU/token throughput, a live native browser session and a full desktop real-model agent turn did not run. Seeded checkpoint/ledger checks are not a real application restart. Native desktop performance and UX are not certified by these tests.
- **External integrations:** HTTP fixtures and actual local stdio MCP processes exercise protocols. Real remote MCP accounts, gated Hugging Face downloads, live OAuth services and the current availability/content of third-party search/weather providers were not certified.
- **Maintenance:** IANA rules are bundled with the pinned timezone dependency; update it when rules change. Fixed aliases such as `EST`, `CET` and `CST` intentionally retain legacy fixed offsets; use IANA regions for seasonal rules.

Existing release optimization (`lto`, `opt-level = 3`), 80 ms stream coalescing and 500 ms persistence checkpoints were already present before this audit. They are not reported as new gains. The large chat/subagent tool loops remain a consolidation candidate, but deleting lines without preserving cancellation, approvals, checkpoints and compatibility would be a regression.
