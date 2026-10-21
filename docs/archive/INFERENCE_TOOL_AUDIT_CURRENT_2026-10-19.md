# Current inference/tool audit checkpoint

Date: 2026-10-19. Scope: LocalLM working tree, including uncommitted changes.
Status: **implementation active; final acceptance incomplete**.

The requested independent audit used Devin CLI 3000.11.92 with the exact model
UID `swe-2-max`. Its [original report](SWE2_MAX_INFERENCE_REVIEW_2026-10-19.md)
and [corroborated findings](INFERENCE_TOOL_AUDIT_2026-10-19.md) remain historical
evidence. Subsequent SWE-2 Max passes and coordinator changes have addressed many
findings. This checkpoint is a current implementation inventory, not a new
completed independent review. The [active plan](INFERENCE_TOOL_IMPLEMENTATION_PLAN.md)
records acceptance work for every A01–A18 and X01–X05 item.

## What the model already has

Shell/Python execution, workspace reads and edits, browser/computer tools, MCP
tools, artifacts, memory, context compaction and delegation already existed.
Availability depends on the preset, selected sources, model support, configuration,
capability switches and approval. Shell can invoke installed Git, language tools,
builds and tests. A missing typed wrapper does not imply execution is impossible.

The new `tool` function is also registered with one required string `input`,
`additionalProperties:false`, normal harness validation/dispatch and structured
confirmation output. Its public contract is covered by harness tests.

## Still missing or awaiting acceptance

| Scope | Current gap | Practical impact / remaining work |
|---|---|---|
| A12 | Capability/constrained-child final acceptance | Per-model controls, source/target pins, Unknown support preservation, native schema requests and full answer post-validation are implemented as a draft. Six offline tests and discovery/probe/UI coverage pass. The foreground/background local/OpenAI/Responses/Claude loopback HTTP fixture (`both_child_drivers_send_native_schema_and_still_reject_invalid_answers`) now executes green in the current environment. Fresh UI captures and independent review remain. See [model capabilities](MODEL_CAPABILITIES.md). |
| A14 | MCP interactive continuation and remote wire-body acceptance | Implemented draft: typed `mcp_input` registry + `resolve_mcp_input` command + `McpInputDialog` for legacy `elicitation/create` and MRTR `inputRequired` continuation, bounded continuation rounds with verbatim `requestState`, `mcp_http.rs` bounded plain-JSON/error/SSE bodies via a public `StreamableHttpClient` wrapper, bounded SSE reconnects (`retry_config.max_times=8` — the SDK's private fatal-error type cannot recognize the wrapper's oversized-event error and its default policy retries forever), `CallUi` authority threaded through parent/PTC/harness/foreground-child paths. 13 protocol-fixture tests + 23 frontend tests pass, and `BoundedHttpClient` is verified over real loopback HTTP (`bounded_http_client_enforces_wire_limits_and_header_parity`: bounded success/error bodies, SSE event bound on POST and GET, session/auth/custom-header parity, JSON-RPC error passthrough, delete-session 405 tolerance). Full preset-catalog connect() integration and final review remain. See [MCP protocol tools](MCP_PROTOCOL_TOOLS.md). |
| X03 | Native model-facing document/PDF/image readers | Implemented draft: `document_inspect`/`document_read` workspace tools with byte-level format detection, no-follow scoped access, SHA-256/untrusted metadata, bounded UTF-8/PDF (lopdf bounded load+extract, text-layer only, ≤8 pages)/Office/PNG-JPEG-WebP handling, artifact-captured image content for vision models only. 9 deterministic tests pass. Final review remains. |
| X04 | Native Responses acceptance and remaining replay lifecycles | Lifecycle gaps now covered as a draft: per-round `replay_answer` scoping, native context preserved on continuations/retries/plan/repair rounds, finish-round anchoring with suppressed-call unknown outcomes, text-parsed-call parity protection, and canonical degradation for mixed program+model audit groups. Native HTTP fixtures, fresh provider UI captures and final independent review remain. See [adapter details](NATIVE_RESPONSES.md). |
| A11 | Deferred catalog final model-round acceptance | Full selected catalog is retained with 64 stable advertised schemas and authorized discovery/invocation. The real HTTP model-round fixture (`real_model_discovers_and_invokes_late_live_mcp_tool_with_original_validation`) now executes green in the current environment: loopback HTTP model + live 70-tool stdio MCP, deferred discovery, pre-RPC argument rejection and stable 64-schema advertising across rounds. Final independent acceptance remains. |
| X02 | Fresh language-server Settings visual proof | Settings → Language servers is beside Bash/Python, as requested. Runtime and frontend tests pass. Fresh desktop/mobile light/dark and feedback-state captures remain pending; the declined preview-origin approval has been respected. |
| All | Final merged native verification and independent adversarial review | Earlier SWE green checkpoints predate later coordinator changes. Loopback HTTP fixtures now execute in the current environment (the earlier socket-bind restriction does not apply here); the merged suite is green at 684. External (non-loopback) provider/remote-MCP endpoints and browser acceptance remain unexercised. Do not treat earlier results as proof of the entire current tree. |

## Implemented work awaiting the final all-scope gate

| IDs | Behavior now present | Evidence / limits |
|---|---|---|
| A01, A16 | Owned asynchronous process start/poll/stdin/stop and prerequisite-aware catalog gating | Real process, timeout, cancellation and descendant-cleanup fixtures; missing runtimes give actionable notices. |
| A02, A06 | Authorized child alias reconstruction and cancellation across blocking operations | SWE fixtures cover foreground/detached paths over loopback HTTP where applicable; final independent review remains. |
| A03, A04, A12 | Full JSON Schema validation and configured native child constraints | Nested validation, strict-schema rejection before network, protocol field mapping, persisted capability provenance and exact parent/child reconstruction. Provider constraints supplement full answer validation; the foreground/background loopback HTTP fixture now executes green here. |
| A05 | Versioned structured envelopes across tool families | Explicit outcome, retryability and side-effect certainty; independent malformed/MCP/unknown-write regressions. |
| A07, A08 | Bounded independent read concurrency and bounded JSON programmatic tool calling | Ordered audit/results, cancellation, runtime branches/loops/transforms, shared pre-dispatch budgets and canonical uncertain-operation replay guards. Later coordinator hardening has targeted tests; final SWE review remains. |
| A09, A10 | Scoped content/glob search and hash-pinned multi-file transactions | Pagination/bounds/path policy, conflict/rollback evidence and opened-handle identity checks. Windows helper type-checks passed; Windows runtime was not exercised. |
| A13 | Bounded cancellable transient inference retries before successful streaming | Retry-After, error classification, credential redaction and no partial-stream replay fixtures — all seven loopback HTTP fixtures execute green in the current environment. |
| A15, A17, A18 | Consistent capability kills, advanced presets and durable research lifecycle controls | Catalog and stale-dispatch gates, persistence, actual progress/pause/resume/cancel behavior and user-control precedence. |
| X01 | Seven typed Git tools | Real temporary repositories, staged-tree pins, source/approval/capability gates, bounded output and exact continuation pins. See [Git tools](GIT_TOOLS.md). |
| X02 | Five typed language-server tools and configuration | Real stdio authority/UTF-16/rename/timeout/cleanup fixtures; hash-pinned rename artifacts and native transaction application. See [language servers](LANGUAGE_SERVER_TOOLS.md). |
| X05 | Structured check execution and test/build reports | Owned execution, Cargo/libtest/Vitest/Jest/TAP parsing, fail-first paging and explicit incomplete/lost-output handling. See [check tools](CHECK_TOOLS.md). |

## Verification provenance

The latest merged native run (pass 6e, adding the `BoundedHttpClient`
loopback wire fixture) finished with **684 passed, 0 failed, 5 ignored** and a
warning-free `cargo check --all-targets`; `npx vitest run` re-verified 595
passed / 1 skipped and `npm run build` passes. Passes 6h–6l completed the
adversarial review sweep across store/artifacts/chat/subagents/ptc/history
internals plus a panic-construct audit with no further defects. Logs and
addenda: `/tmp/locallm-implementation/pass05-report.md`,
`pass06-rust-full-suite.txt`, `pass06-rust-check.txt`,
`pass06-a14-mcp-tests.txt` (47 focused MCP tests).

The earlier merged native run (pass 5, after A14/X03/X04 work) finished with
**681 passed, 0 failed, 5 ignored** in ~63 seconds, a warning-free
`cargo check --lib`, **23** focused frontend ToolControls tests passed and a
clean `tsc --noEmit`. Logs: `/tmp/locallm-implementation/pass05-rust-full-suite.txt`,
`pass05-rust-check.txt`, `pass05-a14-tests.txt`, `pass05-x03-x04-tests.txt`,
`pass05-frontend-tests.txt`, `pass05-typecheck.txt`; the pass-5 report with the
changed-file manifest is `/tmp/locallm-implementation/pass05-report.md`.

SWE-2 Max pass 4 reported **620 Rust tests passed, 0 failed, 5 ignored** in
62.57 seconds. Its exact report is preserved at
`/tmp/locallm-implementation/pass04-report.md`. An intermediate merged run had
**601 passed, 50 failed, 5 ignored** with every failure traced to forbidden
socket creation in this sandbox; the pass-5 run reports no such failures —
the socket-dependent fixtures in that run were already environment-gated.

New pass-5 coverage: 13 MCP protocol-fixture tests (answerable MRTR rounds,
opaque-state echo, continuation cap, decline propagation, legacy elicitation
with/without UI authority, reserved-namespace isolation, oversized catalogs),
9 deterministic document tests (text/PDF page extraction/decompression-refusal/
Office flattening/member bounds/PNG-JPEG-WebP/malformed/oversized/extension
spoofing/path escape), artifact image-capture tests for JPEG/WebP, and 2 new
durable-history tests (finish-round anchoring with suppressed-call unknown
outcomes, mixed program+model canonical degradation) verified end-to-end
through `responses_adapter::request`.

Still restricted in this environment: real TCP/HTTP integration (socket
creation returns `Operation not permitted`), live external providers and paid
API calls, and browser-based visual acceptance (preview origin declined). No
all-scope independent acceptance is claimed; that gate is reserved for a
separate reviewer. One dependency was added for X03: `lopdf = "0.44.0"`.
