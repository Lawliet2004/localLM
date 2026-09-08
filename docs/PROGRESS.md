# Evidence and remaining work

## 2026-09-08
- Empty workspace inspected. Scaffolded official Tauri React/TypeScript template.
- Rust 1.98.1, Node 24.14.1 and npm 11.11.0 available.
- Prior hardware probe: RTX 2050 4096 MiB, Ryzen 5 7535HS, 15.2 GiB usable RAM.
- All acceptance items in SPEC.md remain unverified. Application is not release-ready.

## Verified first native chat slice
- `npm test`: 3 runtime form behavior tests passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: 13 tests passed (runtime validation, GGUF rejection, SQLite persistence/recovery, SSE boundaries).
- `npm run build`: passed; initial application JS approximately 386 kB (119 kB gzip).
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`: passed before MCP dependency work.
- `npx playwright test`: 2 browser tests passed; inspected dark-theme screenshot. Theme switch, catalog filtering and no horizontal overflow at 320/768/1024/1440 verified. This does not verify full accessibility.
- `node scripts/native-smoke.mjs`: actual Tauri IPC -> Rust -> llama.cpp -> RTX 2050 -> streamed answer `42`, then conversation rename and persistence after webview reload. Model load 6554 ms; end-to-end short generation and UI checks approximately 2238 ms. No captured webview errors.
- Runtime and Q6_K downloads completed with pinned SHA-256 checks. Runtime `--list-devices` reported CUDA0 RTX 2050, 4095 MiB total / 3299 MiB free before model load.
- `node scripts/runtime-probe.mjs`: real MiniCPM Q6_K structured `get_weather({city:"Paris"})` followed by synthetic tool result continuation; model incorporated 19 C. Reported short follow-up generation rate 49.8 tokens/s. This is not a full benchmark or live connector test.

## Still required
All full acceptance items in SPEC.md remain open. Connector connection controls are wired to native MCP and OAuth services; skill installation remains unavailable. Next: bounded agent tool loop and approval UI, authenticated-provider verification, then skills/execution. Chat still needs retry, context handling, durable error details, restart acceptance, export verification, broader cancellation/failure tests and live runtime metrics. Model settings need presets, download management, richer GGUF validation and actual placement reporting. No installer/release verification yet.

## Development processes
Vite was started on 127.0.0.1:1420. A debug native app was launched with WebView2 CDP on 9223 for smoke tests; close that debug process before rebuilding the executable. Production launch does not enable CDP. Runtime has been unloaded after the smoke test.

## Verified connector discovery slice
- Native connector UI exposes all 14 presets, API token entry, OAuth sign-in/cancel, connection/disconnection, credential removal and tool schemas.
- `cargo test`: 18 passed. `cargo clippy --all-targets -- -D warnings`: passed after MCP/OAuth/vault integration.
- `npm run build`, 3 frontend tests and 2 browser tests passed.
- `node scripts/connector-smoke.mjs`: real native application connected to public DeepWiki MCP, discovered ask_question/read_wiki_contents/read_wiki_structure, and disconnected. Native screenshot inspected.
- Credentials use AES-256-GCM encrypted files with a Windows Credential Manager master key; cryptographic tampering and large-secret tests passed. Actual account sign-in, refresh/revocation, and OS vault restart tests remain required.
- This verifies discovery only, not chat tool execution or all external services.
## Verified first agent tool loop
- Streamed tool calls assemble across chunks, with up to 8 calls per round, 8 execution rounds, bounded arguments and results, and a maximum of 32 offered tools.
- Connected services can be selected in chat. Each proposed call opens a modal showing the service, real tool name and exact arguments, with single-use Allow once/Deny decisions. Escape denies. Cancellation removes pending approval without executing it.
- Requests/results are persisted as tool messages and rendered in expandable cards. Remote timeout/cancellation reports an unknown outcome and does not automatically retry.
- `node scripts/agent-smoke.mjs`: real MiniCPM Q6_K + native Rust MCP + DeepWiki read_wiki_structure for tauri-apps/tauri. Allowed call returned repository sections (verified in stored JSON); denied call produced a denied record and explanation; cancellation during approval ended with no executed tool record. Three scenarios passed twice after replacing opaque aliases with descriptive tool names.
- `cargo test`: 21 tests passed, including interleaved/malformed tool fragments and single-use approval identity. Frontend production build passed.
- Remaining agent work: individual tool selection for large catalogs, schema validation, context/token budgeting, durable pending-approval recovery, live tool progress, richer error states, full tool history replay, and broader transport failure/hostile-output tests. This slice is not production readiness.
## Verified skill package and guidance slice
- All 13 TrueForge skill paths verified against current upstream Git trees, pinned to immutable revisions, including 94 original files and applicable licenses. Git blob identity and SHA-256 verified during lock generation.
- Rust installs pinned files with streaming byte limits, SHA-256 checks and staging-to-final publication. Path traversal and modified-content tests added. Skills page supports install, verify, inspect package files, activate/deactivate and remove.
- `node scripts/skills-smoke.mjs`: native installation and verified reading of all 13 packages passed. Wiki QA installation, instruction preview, activation persistence after webview reload, removal and reinstallation passed.
- The same native test loaded MiniCPM and asked about the active skill's response format without placing that format in the question. It correctly described the Key Files table, Mermaid diagrams and citation formats, proving guidance reaches the model.
- Native tests: 23 passed. Strict Rust lint and frontend/browser checks passed before the last active-skill indicator UI change.
- Remaining: per-conversation activation, dependency detection/setup, approved script execution, reference-file tools, update/repair UX, install cancellation/progress, crash/disk-full recovery tests, and complete workflows for each skill. Installed guidance is not equivalent to completed execution support.
## Verified workspace file tools
- Added folder selection/persistence and an explicit Workspace files toggle. Workspace tools share the existing single-use approval and audit flow.
- list_files, read_file with line ranges/SHA-256, create_file without overwriting, and create_directory operate through cap-std directory handles. Added strict relative-path/argument validation and bounded data.
- 26 Rust tests passed, including a real Windows junction pointing outside the root: read and create both rejected, outside content unchanged. Strict lint and existing frontend/browser checks passed.
- `node scripts/workspace-smoke.mjs`: real MiniCPM read a fresh random code from a workspace file; denied creation left no file; approved creation wrote exactly the content shown for approval. Verified actual disk bytes, not only the assistant's claim.
- Earlier native attempts exposed model transcription variability (added punctuation and shortened code); the final prompts explicitly requested exact strings and the full prefix. This test does not establish universal model editing accuracy.
- Remaining: existing-file edits with conflict detection/diffs, file browsing/artifact previews, search, per-conversation workspace permissions, execution providers, and further failure/recovery tests. Local process execution is not implemented by these file tools.
## Verified local execution slice
- Python, Node.js and PowerShell interpreters detected/configurable through the Execution page. Local code is an explicit chat tool opt-in and requires approval of its full code; it is clearly described as unsandboxed account-level execution.
- Maximum code 32 KiB, timeout 1–90 seconds, output 64 KiB per stream. Results include exit code, stdout/stderr, error and duration. Unicode, nonzero exits, excessive output and deadlines covered by native tests.
- Cancellation test first exposed a process-wrap 9.1.0 defect allowing descendants to survive. Upgraded to 10.0.0 after inspecting its corrected wrapper lifecycle. Test confirms a real spawned descendant starts, then cannot complete its delayed write after cancellation.
- 29 Rust tests and strict lint passed. Existing frontend and browser checks passed.
- `node scripts/execution-smoke.mjs`: real MiniCPM/native approval -> Python -> data file sum 54, confirmed from persisted stdout/exit status. Denied code made no file. Cancellation after a started marker prevented the script's delayed write and persisted an interrupted tool record.
- Remaining execution work: Daytona, configurable dependencies and skill script workflows, live output/progress, artifacts and previews, per-chat permissions, longer-running jobs, and fuller recovery tests. CPU/memory limits beyond time/output are not implemented for local processes.
## Verified hardware telemetry and loaded settings
- Models page displays NVIDIA device name, driver, device-wide VRAM/utilization, logical CPU count and available physical RAM. Native queries have timeout/output bounds. Missing readings remain unavailable rather than zero. Polling runs only on the visible Models page.
- Runtime status retains the configuration used for the current model load, separately from saved settings. UI describes GPU layers as requested settings, not a fabricated measured layer count.
- 31 Rust tests passed, including multi-device telemetry, unsupported fields and inconsistent driver data. Frontend tests, browser checks, build and strict Rust lint passed.
- `node scripts/hardware-smoke.mjs`: actual RTX 2050 utilization sampled at 21–97% during generation, with 2047 MiB used of 4096 MiB. A second run sampled 69–97%. UI and progress bar inspected in native WebView2. Readings are device-wide, not process-specific attribution.
- Saved CPU-only configuration while GPU model remained loaded, verified loadedConfig still reported -1 GPU layers, then restored saved configuration. No false implication that settings apply without reloading.
- Remaining runtime work includes actual per-layer placement, model-specific memory/timing metrics, download management, context budgeting and broader backend/fallback checks.

### Chat submission and tool outcome correction

Tool audit cards now distinguish `isError: true` results from successful transport completion, while keeping denial and interruption labels. Submission has an immediate in-flight guard and respects conversation loading. Failed sends query persisted messages before restoring the original draft, avoiding duplicate user messages after inference failures; unverifiable persistence prompts reopening the conversation. A newly typed draft is preserved if an earlier submission fails.

Verification: 8 frontend tests pass, including four tool outcomes and duplicate-submit/new-draft regression coverage; TypeScript/production build and both browser tests pass. The persistence reconciliation branch still needs native failure-injection coverage; durable structured generation errors and explicit retry workflows remain unfinished.

### Individual connector tool selection

The chat picker supports individual discovered tools, searchable by connector/name/description, with explicit structured IPC selections (`connectorId`, `toolName`). The model receives only selected tool definitions. Workspace tools count as four and local execution as one toward the 32-tool turn limit. Whole-connector requests remain supported for existing IPC clients, but the UI sends exact selections. Rust rejects stale tool names, deduplicates selections, checks connection health and enforces the limit before message persistence.

Verification: 32 Rust tests and strict Clippy pass; 9 frontend tests, production build and 2 browser tests pass. Regression fixtures cover selecting two exact tools from a 100-tool catalog, unknown tool rejection, and selecting/searching a tool from a 40-tool UI catalog. Native MiniCPM/DeepWiki smoke tests enabled only `read_wiki_structure` and passed real allowed-result continuation, denied-call handling and cancellation. Native stale-selection smoke verified an error and zero persisted messages. Native screenshot inspected at `test-results/native-agent.png` (ignored).

Remaining: selections are currently session state, not persisted per conversation. Account-specific provider compatibility, larger-context management, Daytona, runtime installation and release acceptance remain open.

### Conversation-specific tool persistence

Connector selections and local tool enablement now belong to each conversation in SQLite. Existing chats default to tools off; selecting a chat loads messages and tool settings together, and new chats clear selections. Changes in an existing chat are reflected after successful persistence; sends wait until settings are saved. A foreign-key table removes settings on conversation deletion. Invalid, duplicate and excessive selections are rejected before writes. Settings retain unavailable tool identities for user review rather than silently granting replacements.

Verification: 34 Rust tests, strict Clippy, 9 frontend tests, production build and both browser tests pass. Database reopen tests prove independent settings survive, rejected writes preserve previous data and deletion cascades. `scripts/conversation-tools-smoke.mjs` exercised two different selections through native UI, switching, UI reload and new-chat reset. Real MiniCPM/DeepWiki allow/deny/cancel smoke passed again after persistence integration.

Workspace folder and skill activation are still global settings; this slice persists tool enablement and connector/tool identities only. Full release acceptance remains open.

### Native conversation export

Replaced browser Blob downloads with the native Save dialog and a Rust snapshot/export command. Markdown preserves reasoning, statuses and complete literal tool audits; versioned JSON preserves message fields and tool selections. Writes stage and sync a temporary sibling file before replacement, reporting validation or filesystem errors. The UI handles cancellation, disables export during active operations and displays successful destination paths.

Verification: 36 Rust tests pass; strict Clippy, 12 frontend tests, production build and both browser checks pass. Native `scripts/export-smoke.mjs` compared exported JSON exactly against three persisted messages including one real tool audit, checked Markdown content/reasoning, and verified replacing an existing export. UI tests mock the Save dialog to verify accepted paths, cancellation and write errors. The actual Windows Save-dialog interaction still needs manual acceptance; native file-writing IPC was tested directly. Usage and format details are in `docs/EXPORTS.md`.

### Tool-result history across conversation turns

The model request now restores persisted tool request/result exchanges before each turn's consolidated assistant answer. Previous tool outputs retain the tool role; interrupted or unfinished actions retain an explicit unknown-outcome warning, and denied requests remain denied. Invalid saved audits produce an error before persisting a new user message. Completed assistant prose is included; partial/error assistant prose and saved reasoning are not replayed. Historical call IDs derive from unique audit-row IDs. MCP tool names are now deterministic across selection ordering, with readable names and a hash suffix to distinguish sanitized/truncated identifiers.

Verification: 40 Rust tests and strict Clippy pass; 12 frontend tests, production build and both browser checks pass. Native `scripts/history-smoke.mjs` generated a random verification code in a workspace file, obtained a single approved read, required the first answer to omit the code, disabled tools, reloaded the UI, and asked a follow-up. MiniCPM returned the exact code from saved tool history with no second call. This passed before and after stable alias integration.

The persisted UI schema consolidates all assistant prose for a turn; reconstruction places it after the turn's tool exchanges and reconstructs sequential call/result pairs rather than reproducing the exact original streamed round boundaries. Token-budget/context-overflow management remains unfinished and is more important now that tool results participate in subsequent prompts. Full release scope remains open.

The live DeepWiki allow/deny/cancel regression also passed with the deterministic MCP names.

### User-requested conversation permission modes

Implemented the user's required Ask for approval, Auto-approve reads and Full access controls, visible above chat and persisted with each conversation's tool selection. Ask is the default for new and pre-existing conversations. Auto mode authorizes only native workspace reads/listings; code, writes and connector calls still ask. Full access runs selected tools without prompts. Audit records include the mode and authorization source. A denial blocks further tool execution for that turn, including subsequent calls in the same generated batch; a written response remains possible. Mode changes are disabled while a turn runs.

Verification: 43 Rust tests and strict Clippy pass; 13 frontend tests, production build and both browser checks pass. Native `scripts/permissions-smoke.mjs` verified Ask/read denial, automatic read, Auto/write denial with no created file, Full/file creation with exact bytes, and Full/Python execution returning 42 without prompts. It checked saved audit provenance, reload persistence and new-chat reset. `scripts/permissions-connector-smoke.mjs` verified Auto asks/denies a real DeepWiki call and Full performs the same selected call without prompting, returning actual repository data. Screenshot `test-results/native-permissions.png` inspected. See `docs/PERMISSIONS.md` for precise scope.

During verification, one 512-token run exhausted its budget in reasoning before requesting a tool. Smoke fixtures temporarily use 2048 tokens and temperature 0, restoring prior preferences afterward. Response-limit feedback remains an outstanding reliability issue. Context-token preflight research located the pinned llama.cpp `/v1/chat/completions/input_tokens` endpoint; it is not implemented yet. The larger application goal remains active.

### Durable generation errors and response-limit handling

Generation now checks the runtime's finish reason before accepting a response or executing assembled tool calls. A `length` finish produces an actionable response-limit error; partial text/reasoning remain saved, and unfinished tool calls are not executed. Runtime content-filter termination is also reported. Terminal generation state and its error text are saved together. SQLite migration adds a nullable message error column to existing databases; the UI renders the saved error after reopening, and JSON/Markdown exports include it.

Verification: 45 Rust tests and strict Clippy pass; 14 frontend tests, production build and both browser checks pass. `scripts/response-limit-smoke.mjs` used the real loaded MiniCPM runtime with a one-token response limit, verified error status and identical persisted error text, reloaded the UI and confirmed the error remained visible. Original preferences were restored afterward. Database reopen tests verify partial content, reasoning and error text survive. Context-token budgeting and proactive overflow handling remain open.

### Tokenizer-based context checks

Added authenticated preflight using the pinned runtime's `/v1/chat/completions/input_tokens` endpoint. The exact generation payload is counted before saving a new user message; prompt tokens plus the requested response must fit the loaded context. Each subsequent tool round is counted again. Oversized initial sends restore the draft without persisting messages. Overflow after a tool preserves completed audits and saves a generation error. No history is silently discarded. Count responses and request time are bounded; invalid/unsupported count responses fail visibly instead of using a character approximation.

Verification: 47 Rust tests, strict Clippy, 14 frontend tests, production build and both browser checks pass. Native `scripts/context-smoke.mjs` rejected a 12,038-token prompt plus 2,048 response tokens against an 8,192-token loaded context, confirmed zero saved messages and restored draft, then successfully sent a short message. An approved 90,000-byte workspace read subsequently triggered a context error before the next model round; its successful result and audit remained saved. The corrected native test passed twice. HTTP fixtures verified authentication, exact Unicode/tool payload forwarding and rejection of negative, missing or oversized count responses.

`docs/CONTEXT.md` records behavior and pinned upstream source references. Automatic compaction, live composer usage, context selection and full release acceptance remain open.

Live DeepWiki Auto/denial and Full-access call regressions also passed with token preflight enabled.

### Conversation draft recovery

Unsent text now belongs to its conversation rather than the mounted chat component. Drafts persist in the desktop WebView's local storage and remain available when switching chats, navigating to another page, or reloading. The new-chat draft has its own slot; conversation creation transfers it to the assigned conversation ID. Submitted text clears through the existing send flow, while rejected sends restore it. Deleting a conversation clears its draft. Storage failures retain the current text in session memory and report that it could not be saved.

Verification: 16 frontend tests, production build and both browser checks pass. Native `scripts/drafts-smoke.mjs` verified two independent unsent drafts, a separate new-chat draft, reload recovery and navigation away/back. Fixtures restored the prior new-chat draft and removed their temporary conversations. Rust was unchanged in this slice.

Drafts are local WebView data, separate from SQLite conversation exports. Crash recovery during the interval between submitting a draft and backend acceptance remains to be hardened; this slice verifies unsent draft persistence, not transactional draft-to-message handoff.

### Atomic turn acceptance

The accepted user prompt, streaming assistant record and initial conversation title now commit in one SQLite transaction. An injected assistant-insert failure verifies that no user row or title change remains; unknown conversations cannot create orphan rows. This does not yet make the WebView draft-to-backend handoff crash-safe.

Verification: 48 Rust tests, strict Clippy and the native debug build pass. The rebuilt desktop application passed the response-limit smoke test against the real model, persisting both message records and the terminal error, then showing the same error after reload.


### Active skill reference access

Active skills now expose one shared skills_read_file tool with their pinned package inventories. The model can inspect supporting references and script source through bounded, numbered pages, with hash verification and capability-scoped file access. The active set is fixed for the turn. The reader occupies one visible tool slot and follows permission policy: Ask and Auto-approve both prompt; Full access does not. Reading does not execute scripts.

Verification: 51 Rust tests, 17 frontend tests, production frontend build and both browser checks pass. Native scripts/skill-reader-smoke.mjs passed all three modes against MiniCPM, reading the exact first three lines of the Jupyter notebook quality checklist and producing the expected answer. The Windows junction escape test passed. Active skills, preferences and temporary chats were restored/removed by the smoke fixture. Dedicated package script execution, dependencies, updates and other release acceptance remain open; see docs/SKILLS.md.


### Desktop launch and window persistence

Added the official Tauri single-instance and window-state plugins. Duplicate launches exit and restore/focus the existing main window. Size, position and maximized state persist; hidden/minimized state is not restored. No frontend permissions were added.

The native debug build and scripts/window-smoke.ps1 passed: exact window bounds survived restart, maximized state survived another restart, a second process exited while the original PID remained, and a minimized window was restored. Original geometry was restored after testing. Physical disconnected-monitor behavior and installer acceptance remain unverified. See docs/DESKTOP.md.


### First optimized Windows installer verification

The NSIS release build completed in 6m18s. Installer size is 3,412,952 bytes; the application executable is 10,120,704 bytes. The artifact is unsigned and excludes the separately prepared runtime/model. docs/RELEASE.md records its source commit, SHA-256 and verification limits.

Silent current-user installation into a temporary folder succeeded. The installed bundled frontend loaded the actual model, answered 17 + 25 with 42, persisted a complete response and showed no JavaScript errors. Silent uninstall removed the installation and registry entry while leaving the database hash unchanged. The fixture now waits for NSIS's temporary uninstaller to finish cleanup. The debug app was reopened afterward. Production npm dependencies reported zero known vulnerabilities. The starter README was replaced with setup, privacy, troubleshooting and remaining-scope documentation.

This proves the packaging/desktop checklist item, not production readiness. Clean-profile setup, signing, updates, account-specific connectors, downloads, execution providers and other full acceptance requirements remain open.


### Runtime-reported GPU layer placement

Models now shows actual layer counts from the runtime startup summary alongside requested settings. Parsing is bounded to 2 MiB and rejects malformed, impossible or missing evidence rather than inferring success from configuration. The pinned runtime needs trace verbosity 4 for library startup diagnostics; debug verbosity 5 remains disabled. Stopping/exiting clears the report.

Verification: 52 Rust tests, 17 frontend tests and the frontend production build passed. Strict Clippy passed after replacing a forward full scan with reverse lookup. Native scripts/offload-smoke.mjs verified 43/43, 5/43 and 0/43 for all-layer, five-layer and zero-layer requests, including visible UI checks. Saved configuration and loaded/stopped state were restored. The screenshot was inspected. This reports layer placement; per-device allocation and KV placement reporting still remain. The existing installer predates this change.


### Bounded runtime diagnostics

The runtime no longer writes directly to an unbounded file. Shared stdout/stderr draining caps each load's log at 8 MiB including a truncation marker, then discards further output while keeping pipes flowing. Write failures likewise do not stop draining. Shutdown waits a bounded interval for drain tasks after terminating the child. Startup diagnostics are retained; later diagnostics after the cap are intentionally unavailable.

Verification: the full 53-test Rust suite passed after implementation, then both logging tests passed after adding a write-failure fixture (54 total tests now). Tests stream 200,000 bytes through tiny pipes into a small cap, verify a single marker and exact size, verify reset on new load, and verify read-only-file failure cannot deadlock a producer. Strict Clippy and native build passed. Rebuilt-app offload checks again reported 43/43, 5/43 and 0/43; the real response-limit generation/reload check also passed. Frontend code was unchanged. Full release scope remains active.


### Daytona API contract and permission copy audit

Audited remaining SPEC items and selected Daytona as the next missing execution provider. Inspected official platform/toolbox OpenAPI schemas and recorded hashes, request requirements, unresolved proxy-auth/timeout details and the durable cleanup design in docs/DAYTONA.md. No cloud resources or account calls were made. Implementation and live-account evidence remain outstanding.

Corrected the Execution page's outdated claim that all runs require approval: it now accurately explains Ask/Auto-approve versus Full access. The production frontend build passed. This turn establishes the next implementation contract; it does not claim Daytona is available.


### Daytona native transport

Implemented a low-level Rust client for sandbox creation, inspection, deletion and code-run. Requests have bounded timeouts/responses, redirects disabled, validated identifiers and a strict hosted toolbox destination. Creation carries an operation ownership label and explicit private/resource/lifetime settings. Remote error bodies are omitted from errors. The client is not yet a selectable chat tool; durable ownership/cleanup, polling, vault/UI integration and account testing remain required.

Verified current SDK source at daytona/clients revision 246056df8886a020396cb46ad17c1d8b93653648 for seconds-based execution timeout, Bearer configuration and toolbox URL construction. Three targeted Rust tests passed using local TCP fixtures for authentication, creation payloads, response bounds, 404, redirects and validation. Strict Clippy passed (57 tests now exist overall). No real Daytona resources were created and no account credentials were used.


### Durable Daytona ownership records

Added a SQLite journal independent of conversation storage, initialized by native startup, plus a read-only pending-operations command and Execution-page recovery display. Records retain ambiguous creation, sandbox identity and cleanup errors across restart; reassignment and mismatched-scope acknowledgement are rejected. No cloud calls are made by this slice.

Two journal tests pass (59 Rust tests now exist), all 19 frontend tests pass, strict Clippy, native build and frontend build pass. The rebuilt native app returned an empty journal and opened Execution without an alert. Lifecycle orchestration, credential settings, automatic cleanup recovery and the executable Daytona tool still remain.


### Ownership-checked Daytona cleanup recovery

Implemented the recovery engine over the native transport and durable journal. It validates credential scope, remote name/ownership label and known sandbox ID before deletion; associates discovered identity before mutation; and only removes ownership after confirmed absence. Unknown creation with no visible sandbox remains recorded. Cleanup has a 60-second deadline and retains errors for retry.

All six Daytona tests passed before adding the cancellation test; both final cleanup tests and strict Clippy passed afterward (61 Rust tests now exist). Fixtures cover success, mismatched ownership, server failure, unresolved creation, wrong scope and cancellation during deletion followed by database reopening. No real cloud calls occurred. Credential UI, recovery-button/automatic startup wiring and the cloud execution lifecycle are still incomplete.


### Daytona credential settings and manual recovery

Added native encrypted-key save/presence/forget commands and a manual retry-cleanup command. Execution now provides password input, explicit saved-versus-verified status, forgetting and per-record cleanup buttons. Pending ownership blocks credential replacement/removal. Daytona and MCP share the same vault instance to serialize key operations. Cloud cleanup is serialized and excluded during model operations.

All 19 frontend tests, strict Clippy and production frontend build pass. The final native build passed; scripts/daytona-settings-smoke.mjs verified save, ciphertext absence of the fixture key, input clearing, presence after reload and forget twice, including after the shared-vault change. No account/cloud requests were made. An initial native attempt launched the old executable during compilation; it was closed, the build completed, and the test reran successfully. Key rotation with pending operations, automatic recovery and the cloud execution tool remain open.


### Daytona execution lifecycle

Implemented the coordinator for journal-before-create, identity capture, bounded readiness, single code submission and ownership-checked cleanup. A cancellation sender lets a detached worker finish identity capture and cleanup after the caller disappears. Execution and cleanup errors are returned separately, and nonzero code exits are represented as failures with retained output.

The targeted async lifecycle test passed for normal completion, nonzero exit and caller cancellation, verifying one submission and eventual journal clearance after deletion. Strict Clippy passed (62 Rust tests now exist). This is a fake-remote lifecycle test, not live account evidence. The coordinator remains to be wired to model tool selection/approval, and automatic startup recovery is still outstanding.


### Daytona chat permissions and startup recovery

Connected the cloud execution coordinator to a separately selected chat tool. Ask and Auto-approve reads show code and cost scope before creation; Full access follows the existing native policy. Credential mutations are excluded during model operations. The native denial smoke passed in both prompting modes with zero resource journal records. Production frontend build, 19 frontend tests, 62 Rust tests and strict Clippy passed for chat wiring.

Added one launch-time cleanup pass over pending ownership, serialized with other cloud operations. Missing/mismatched credentials retain actionable errors; one remote failure does not prevent later cleanup. No code or creation is replayed. The new mixed-scope recovery regression and strict Clippy passed. Live cloud execution and native restart recovery remain unverified; no paid cloud calls were made.

The Execution page now refreshes the pending journal every five seconds while mounted, without overlapping polls, and refreshes after credential/cleanup actions. Journal errors have their own visible state and clear after a successful refresh. A fake-timer regression verifies completed cleanup disappears and unmount stops polling. All 20 frontend tests and the production frontend build pass. Native debug rebuild succeeded; the credential UI smoke passed after relaunch with encrypted save, reload persistence, clearing and forget, with zero cloud calls. This confirms startup with an empty journal, not live pending-resource recovery.

### In-app runtime diagnostics

Added Models & runtime > Diagnostics with a refreshable, text-only view of the last 64 KiB of the fixed application runtime log. Native reads are bounded even if the file grows, missing logs produce an empty state, and invalid UTF-8 is displayed lossily. Truncation and read failures are explicit. No caller-supplied path is accepted.

Three runtime-log tests, all 21 UI tests, strict Clippy, production frontend build and native debug build passed. The native diagnostics smoke displayed actual runtime output and refreshed without errors; its screenshot was inspected. The first smoke attempt used an incorrect navigation label and timed out; correcting it to the actual Models & runtime label passed. This adds diagnostics visibility; it does not complete runtime download management or the release acceptance checklist.

### Verified download core

Added a streaming downloader that checks pinned byte length and SHA-256 before publishing a temporary file without overwriting an existing destination. It bounds received bytes, reports received-byte progress, detects a stalled stream, flushes before publication and removes temporary files on cancellation/error. Local HTTP fixtures cover valid download, hash and size mismatch, existing destination preservation, cancellation before transfer and cancellation during a stalled partial transfer. Both tests and strict Clippy pass; an initial Clippy warning in the test fixture was corrected to assert a nonempty request read.

This core is not yet exposed through IPC or the model setup UI. Pinned asset catalog integration, disk-space preflight, production HTTP redirect/timeouts, runtime archive extraction, installation state, UI progress/cancellation and native acceptance remain required. No large downloads were started; available C: space was about 3.1 GiB. The existing preparation script contains the pinned model/runtime hashes; a browser attempt to reopen the pinned model page was unavailable and supplies no new source evidence.

### Download storage preflight and pinned model catalog

Added destination-volume free-space checks via the Windows API, using bytes available to the current account and a 256 MiB reserve. The downloader refuses insufficient space before creating a temporary file or contacting the service. Added a production HTTPS-only client with bounded redirects, connection and total transfer time. The model catalog fixes the known MiniCPM5 Q6_K URL, byte length and hash; a native read-only metadata command reports its application-managed destination, existence (not verification), available space and requirements.

All three download regression tests and strict Clippy pass. New coverage checks real Windows space queries, a missing directory, arithmetic overflow, insufficient-space refusal without file creation and HTTP rejection by the production client. No model transfer was started. Installation commands, progress UI, existing-file verification, runtime archive handling and native catalog acceptance remain open.
