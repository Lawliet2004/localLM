# Inference and model-facing tool audit

Date: 2026-10-19. Target: the current LocalLM working tree, including uncommitted work.

## Method and limits

The requested independent review completed successfully (exit code 0, empty stderr) with installed Devin CLI 3000.11.92, selecting the exact account-listed model UID `swe-2-max`:

```sh
devin --model swe-2-max --permission-mode auto \
  --respect-workspace-trust false \
  --prompt-file /tmp/locallm-inference-audit/prompt.txt -p
```

The prompt requested a read-only source audit, no application execution, no dependency installation, no credential inspection, and no subagents. The independent review is accompanied by direct source verification of the findings below. This is a static audit: behavior described as a defect or limitation is inferred from current code, not established by exercising a live model or connected service. Existing tests and comments are evidence of intent, not proof that integrations work today. Product configuration was not inspected; a feature's actual availability in a particular running conversation remains conditional on its selected model, sources, preset and settings.

The [original SWE-2 Max report](SWE2_MAX_INFERENCE_REVIEW_2026-10-19.md) is preserved separately with verification notes. This reviewed report combines the corroborated findings with direct source findings; it excludes overbroad claims about foreground MCP access, PowerShell support and unmeasured token-estimation error. No runtime code was changed for this audit.

## Conclusion

LocalLM already has the core execution tools needed for coding: shell commands, code execution, line-based file reads, unique-match file edits, directory listing/creation, search, browser/computer automation, and MCP connectors. The largest opportunities are process lifecycle management, consistency between parent and child inference loops, reliable result contracts, and making existing capabilities usable through the model-facing schemas.

Adding many specialized tools would not address those problems by itself. Git commands, test runners, dependency managers and file operations are already possible through approved shell execution. Dedicated tools for these activities should be justified by better structure, narrower permissions or fewer model mistakes.

## What the model can already use

| Surface | Model-facing tools or behavior | Gates and limitations |
|---|---|---|
| Workspace | `workspace_list_files`, `workspace_read_file`, `workspace_create_file`, `workspace_create_directory`, `workspace_edit_file` | Workspace source must be selected and supported by the preset. Edits require a fresh SHA-256 and a unique old-text match. Reads support line ranges. |
| Code execution | `run_code` in presets offering that alias; otherwise `local_run_code` | Selected execution source, workspace and available configured interpreter. Python source or script, arguments, stdin, cwd, bounded output and generated artifacts already exist. |
| Shell | `local_execute_command` | Execution source, workspace and enabled configured shell. Can run Git, builds, tests and installed CLIs. This is foreground execution with bounded duration/output. |
| Persistent model shell | `terminal_create`, `terminal_send`, `terminal_close` | Offered by selected presets; send waits for a completion marker. This is distinct from the user's interactive PTY. |
| Research | Weather, web search, URL reading/fetching, saved passage open/find; AREX search/visit/context/finish | Presets, offline/web settings, weather configuration, model adaptation and approval. |
| State and delegation | Todos, goals, memory, subagents, messaging/interruption, workflows, compaction and artifacts | Many are present only in advanced presets. Background children have additional restrictions. |
| Browser/computer | Browser catalog and computer actions, including screenshot feedback | Selected source, automation enabled, executable/environment availability; computer use requires image support. |
| Connectors and skills | Selected MCP tools and active skill reader/instructions | Live connection, selection, preset and tool-support gates. |
| Echo | `tool` | Standard, Creator and Code; echoes a string with structured confirmation. It does not add an execution capability. |

Evidence: `src-tauri/src/workspace.rs:515`, `connectors.rs:332`, `execution.rs:374`, `execution.rs:394`, `harness.rs:69`, `presets.rs:48`, and `chat.rs:788` (paths in this report are under `src-tauri/src/` unless stated otherwise).

The assembly path intersects requested sources with preset sources, loads selected MCP/native tools, applies configuration and model support, adapts AREX tools and freezes the catalog. Calls then pass through parsing, offered-tool lookup, argument preparation/validation, approval, dispatch, bounded result storage and continuation. See `chat.rs:788`, `chat.rs:1115`, `chat.rs:2254`, `connectors.rs:323`, `tool_validation.rs:7`, and `chat.rs:2608`.

## Priority findings

Priorities here are implementation order, not vulnerability severity. P1 = high practical impact; P2 = useful next improvement; P3 = optional expansion.

### 1. P1 — No first-class asynchronous process tools

**Status:** Missing model-facing lifecycle capability.

`local_execute_command` runs to completion/timeout. `terminal_send` waits for a sentinel and closes the session on timeout or output overflow (`execution.rs:394`, `sandbox.rs:177`). The interactive PTY exists for the user (`terminal_ui.rs:1`), but the model is not offered its session control API.

**Impact:** A build, test suite, development server or interactive CLI cannot be started, yielded, polled and supplied input through a consistent model tool protocol. Shell backgrounding/log files are possible workarounds, but lack first-class ownership, status and cleanup.

**Smallest addition:** `process_start({command,cwd,yield_ms})`, `process_poll({id,offset,max_bytes})`, `process_write({id,input})`, and `process_stop({id})`. Return a stable process ID, running/exited state, exit code, output cursor and truncation metadata. Bind IDs to conversation/workspace and preserve current approval rules.

### 2. P1 — Background children lose execution/tool capabilities

**Status:** Existing feature with incomplete reconstruction; some restrictions are intentional.

`subagents.rs:538` rebuilds workspace tools, one code tool, the clock and skill reader, then filters by parent aliases. It does not rebuild the command tool, browser/computer tools, MCP peers or the harness catalog. Its code tool defaults to `local_run_code`; the parent may have renamed that tool to `run_code` (`chat.rs:894`, `connectors.rs:332`, `chat.rs:1592`). Alias filtering can therefore remove code execution as well. Although detached harness handlers exist at `subagents.rs:69`, the reconstruction shown does not populate harness tools for dispatch.

**Impact:** A task delegated to a background child can lose precisely the tools needed to complete it. Foreground inheritance is broader and must not be confused with background reconstruction.

**Smallest correction:** Reconstruct a serializable capability manifest with canonical IDs and public aliases, include the shell where authorized, and advertise explicit limitations before scheduling. Keep live MCP/automation restrictions until their ownership model supports delegation.

### 3. P1 — Child output-schema validation is incomplete

**Status:** Implemented contract with insufficient validation.

`subagents.rs:504` parses a JSON object and checks required top-level keys. It does not enforce property types, nested schemas, enums, ranges or extra-property rules.

**Impact:** A child can claim schema compliance while returning values that downstream code cannot consume.

**Smallest correction:** Reuse the existing offline JSON Schema validator, validate the schema at request time, and return a structured mismatch. Example regression: `{answer:123}` must fail a schema requiring a string answer. Provider-native constrained output is a separate enhancement.

### 4. P1 — Structured execution schema exists internally but is unexposed

**Status:** Existing backend capability absent from the public schema.

`CodeRequest` accepts `result_schema` (`execution.rs:247`), and execution validates it (`execution.rs:999`). The public definition at `execution.rs:374` omits that field and disallows additional properties. Normal `prepare_arguments` consequently rejects a model attempting to use it (`connectors.rs:323`).

**Smallest correction:** Advertise `result_schema` in the code tool schema and constrain/validate it using the existing validator. Preserve the distinction between process logs and structured result data.

### 5. P1 — Error contracts differ across tool families

**Status:** Partial standardization; no universal result boundary.

Execution builds a versioned envelope (`execution.rs:1012`); many harness/workspace tools return plain JSON; common errors use `{isError,message}`; MCP returns its protocol result (`connectors.rs:419`); some paths use `{isError,error}` (`chat.rs:2595`). Continuation reads a top-level `message` and classifies free-text errors (`continuation.rs:23`, `continuation.rs:145`).

**Impact:** A timeout represented in `error.message` can lose its ambiguous-outcome classification when continuation sees only `Tool failed.`. Mixed shapes also complicate reliable recovery and audit interpretation. The envelope module's universal-contract comment is stronger than current integration.

**Smallest correction:** Normalize every result once at dispatch, preserving raw data. Carry typed status, error code, retryability, side-effect certainty and artifacts; use those fields for continuation. Never automatically replay uncertain writes.

### 6. P1 — Subagent interruption is not observed during stream collection

**Status:** Parent/child lifecycle inconsistency.

The parent selects cancellation against network/stream waits (`chat.rs:1730`, `chat.rs:1752`). Child `collect_stream` accepts no interrupt handle and waits for chunks with a 300-second idle timeout (`subagents.rs:1133`). Child loops check their interrupt flag around model/tool rounds, rather than inside that collector.

**Impact:** Interruption can be delayed while a child waits on or consumes a model response.

**Smallest correction:** Pass a cancellation receiver through context counting, model request, streaming and tool execution; select cancellation in each awaited operation. Keep interrupted-output and uncertain-side-effect reporting explicit.

### 7. P2 — Multiple calls parse correctly but execute sequentially

**Status:** Missing scheduler optimization, not missing multi-call parsing.

The main loop awaits tools inside `for mut call in calls` (`chat.rs:2254`). PTC also iterates sequentially (`harness.rs:803`). Existing parallel workflows concern child agents, not concurrent calls in one model batch.

**Impact:** Independent reads/searches incur the sum of their latencies.

**Smallest correction:** Bounded concurrent scheduling for known independent reads, preserving call IDs and deterministic audit/result ordering. Retain ordered execution for dependent operations and mutations. No need to add an unrestricted parallel execution tool.

### 8. P2 — PTC is a declared-step batch, not an executed program

**Status:** Implemented subset with a broader description.

`harness.rs:782` checks an optional program's size and `sdk_` call names, then executes the separately supplied steps. It does not evaluate the program, use returned values in conditionals/loops, or transform intermediate results. `presets.rs:197` emits SDK declarations with `unknown` arguments/results.

**Impact:** The advertised programmatic workflow cannot express real runtime dataflow. Plain multi-step batching remains useful.

**Smallest correction:** Describe it accurately as batching first. If needed, add a bounded declarative dataflow format, or an isolated program executor with explicit tool RPC and per-call approvals. Do not evaluate arbitrary code in the host agent process.

### 9. P2 — Search lacks model-facing scope, regex and pagination

**Status:** Search exists; advanced structured search is missing.

Harness `file_search` accepts only a query (`harness.rs:586`), uses fixed-string matching, limits scanned files/hits/bytes, and returns one matching line per file (`sandbox.rs:558`). A separate filename search exists in `workspace.rs:474` and the UI, but is absent from the five workspace tool definitions at `workspace.rs:515`.

**Impact:** The model cannot efficiently narrow a large project by path/glob, retrieve multiple matches per file, choose regex/case behavior or continue a result page. `rg` through shell is already a workaround when authorized and installed.

**Smallest addition:** Extend a content search with path/glob, fixed/regex mode, context lines, cursor and explicit incomplete-result metadata; expose bounded filename/glob search separately. Keep existing sensitive-path policies.

### 10. P2 — No first-class atomic multi-file patch or model file mutation API

**Status:** Native single-file editing exists; broader structured operations are absent.

The workspace catalog exposes unique replacement, creation and reads (`workspace.rs:515`). UI editor mutations are separate IPC actions (`editor.rs:364`). There is no advertised multi-file patch, rename or delete tool in that catalog.

**Impact:** Larger refactors require many model calls or shell workarounds. Rename/delete/Git are not impossible; approved shell can perform them.

**Smallest additions:** `apply_patch` with expected file hashes and all-file preflight, then explicit `move_file`/`delete_file` only if narrower permission and review semantics justify them. Preserve conflict detection and readable diffs.

### 11. P2 — Eager catalogs have a hard 64-tool ceiling

**Status:** Discovery exists for connector loading, not model-directed discovery.

MCP discovery pages tool lists with bounds (`tool_discovery.rs:5`), while assembled inference rejects more than 64 tools (`chat.rs:917`, `chat.rs:1097`). No model-facing tool-search/deferred-schema mechanism is advertised.

**Impact:** Larger connector sets are rejected or require manual selection; all selected schemas consume context.

**Smallest addition:** `tool_search({query})` over already authorized connected catalogs, returning a small set of exact tool definitions. Introduce it only after measuring schema-token pressure; dynamically changing schemas must respect existing frozen-catalog and local prompt-cache assumptions.

### 12. P2 — Provider capabilities are too coarse for inference controls

**Status:** Multi-provider inference exists; richer negotiation is missing.

`RemoteModel` stores context/output limits, tool support and image support (`providers.rs:55`). Reasoning behavior is selected using name lists (`inference.rs:118`, `claude_adapter.rs:215`). The main request builder does not expose a general output-schema constraint (`chat.rs:3002`). Remote context checks use estimated tokens (`inference.rs:407`).

**Impact:** Compatible endpoints with different supported controls can receive defaults that are suboptimal or unsupported; newly named reasoning models do not automatically gain known controls. Structured JSON generation and precise remote preflight counting are not generally available.

**Smallest correction:** Explicit per-provider/model capabilities for supported reasoning controls, token-limit field, structured output and tool-choice modes. Add constrained output/token counting where a provider supports them, retaining honest estimates elsewhere. Unknown capabilities should remain unknown.

### 13. P2 — No bounded inference retry policy for transient HTTP failures

**Status:** Explicit current behavior, rather than a silent failure.

Provider errors state no retry was attempted for 429 and 5xx (`providers.rs:464`); inference streams return those errors (`inference.rs:423`). Download and web-fetch retry logic elsewhere does not provide inference retries.

**Smallest addition:** Cancellable, bounded exponential backoff with jitter and `Retry-After` for pre-stream transient inference errors. Exclude authentication, invalid requests and partial-stream failures by default. Never extend this automatically to side-effecting tool calls.

### 14. P2/P3 — MCP is tools-focused and additional-input flow is unsupported

**Status:** Tool calls work; broader protocol features are unimplemented in the inspected path.

`connectors.rs:419` uses `call_tool_once`; non-complete results return an explicit unsupported-additional-input error. No model-facing resource listing/reading or prompt browsing is advertised by the harness.

**Impact:** Servers requiring an additional interactive exchange cannot complete through this call path. Resource/prompt-only integrations lack a first-class model surface.

**Smallest additions:** Implement a user-mediated additional-input round trip where needed, plus `mcp_list_resources`/`mcp_read_resource` only for services that require them. Preserve connector authorization and content bounds. Ordinary MCP tools are already present.

### 15. P1 — General capability disable flags do not consistently gate inference

**Status:** A saved control is not uniformly enforced by the runtime assembly path.

`capabilities.rs:260` saves global disabled capability IDs, and `capabilities.rs:195` provides the check. Direct call-site inspection finds that check used for `system_time` (`chat.rs:1064`) and compaction (`compaction.rs:123`). Workspace/execution source assembly, skills and advanced harness tools use their own selection/settings checks rather than the corresponding global capability flags (`chat.rs:788`).

**Impact:** Disabling a capability through this registry can show an off state without removing the corresponding tool family from a later inference catalog. This does not mean source selection or execution approval is bypassed; those separate gates still exist. No running configuration was tested.

**Smallest correction:** Apply one capability-to-tool/source mapping during assembly and recheck it at dispatch. Add a regression for each disable flag showing both catalog omission and rejection of stale calls. Resolve the documented capability dependencies rather than treating the registry as display-only metadata.

### 16. P2 — Worker-dependent research tools are advertised without a Node availability gate

**Status:** Tool availability and runtime prerequisites are inconsistent.

`web_search.rs:70` rejects all worker requests when `execution.node_path` is empty, including weather and saved-document open/find requests. Harness assembly checks web/offline/weather settings but does not apply an equivalent Node prerequisite (`chat.rs:885`).

**Impact:** The model can be offered tools that are guaranteed to fail with the current configuration. This is not evidence that Node is missing on this machine; settings were not inspected.

**Smallest correction:** Validate required worker/runtime availability during assembly and emit an actionable unavailable-tool notice. Preserve paths that truly can work without that runtime if those are separated later.

### 17. P2 — Advanced tool presets exist but are absent from the current UI selector

**Status:** Implemented but unexposed through the normal selection UI.

The backend defines Code/PTC, Minimal and Creator profiles (`presets.rs:48`). The frontend radio list offers Standard, Chat, Research and Coding only (`src/components/ToolControls.tsx:324`).

**Impact:** Users cannot normally select the profiles containing PTC, benchmark terminal behavior and broader orchestration. Backend APIs still exist, so these features are not missing implementations.

**Smallest correction:** Decide which advanced profiles are supported product features and expose those choices, or explicitly document them as developer-only. Do not add duplicate execution engines.

### 18. P2 — Model-facing research lifecycle controls only record events

**Status:** Registered tools have narrower behavior than their descriptions suggest.

The common `research_pause`/`research_resume`/`research_cancel`/`research_progress` branch returns `taskAction` and emits a `research_task` event (`harness.rs:681`). It does not directly change active run control or produce a substantive progress report. The UI run-control implementation is separate.

**Impact:** A model calling pause/cancel does not itself stop the current loop through this dispatch branch, and progress returns an action label rather than completed/remaining work.

**Smallest correction:** Wire each tool to the existing durable run-control/progress implementation, or change descriptions and output to say that only a request was recorded. Define resume authorization so model output cannot override a user's pause/cancel.

## Optional enhancements, not baseline execution blockers

- Typed Git status/diff/stage/commit tools: Git UI operations exist (`workspace_ui.rs:319`), and shell Git works. Typed tools could reuse staged-tree confirmation and structured diffs; they are not required to make Git possible.
- LSP diagnostics/symbol/reference/rename tools: useful for code navigation and accurate refactors; not required for the current text-and-shell workflow.
- Native local document/PDF/image reading: document retrieval is bounded, text-focused and explicitly scoped (`document_index.rs:1`). Browser/web extraction and shell libraries cover some use cases; a generic local binary reader needs its own format/size contract.
- Native additional inference protocols: the backend enum supports local/OpenAI-compatible, native Claude Messages and subscription paths (`inference.rs:123`). Other providers may work via compatibility endpoints, but protocol-specific features require adapters. Do not label them wholly unsupported without testing the endpoint.
- Dedicated test/build tools: optional wrappers around the existing shell. Prefer a structured test report and process lifecycle over one tool per package manager.

## False positives excluded

This audit does **not** claim that LocalLM lacks shell execution, code execution, file editing, search, browser automation, computer screenshots, tool argument validation, approvals, compaction, artifact pagination, MCP tools, usage metrics, cancellation of the parent loop or subagents. All have implementations. Nor does an opt-in setting or a deliberately narrow preset establish a defect.

No benchmark was run. Throughput/latency benefits of parallelism and deferred loading are predictions that require measurement on the actual local model and hardware. Adding a tool name alone will not give a model a new inference capability.

## Suggested implementation order

1. **Contract corrections:** enforce capability disable flags; expose execution `result_schema`; fully validate child output schemas; normalize results/errors and continuation decisions; fix child alias reconstruction; make child cancellation responsive. Add focused regressions at the public tool boundary.
2. **Execution lifecycle:** add start/poll/write/stop process tools with conversation ownership, bounded incremental output and cleanup. Verify a long build, a server, cancellation, overflow and process-tree termination.
3. **Coding efficiency:** scoped/paginated search, atomic multi-file patching, and bounded concurrency for independent reads. Measure model round count and time to completion.
4. **Inference scaling:** richer provider capabilities, constrained output where supported, safe transient retry, and model-directed tool discovery if catalog pressure warrants it.
5. **Optional expansion:** real programmatic dataflow, selected MCP interactive/resource flows, typed Git/LSP/document tools based on concrete product needs.

## External research used as design context

- [Devin CLI model selection](https://docs.devin.ai/cli/models): the installed account catalog, rather than a remembered alias, supplied the exact `swe-2-max` UID.
- [Anthropic: advanced tool use](https://www.anthropic.com/engineering/advanced-tool-use): describes deferred discovery, code-driven tool orchestration and examples. These are design references; its reported performance figures were not measured on LocalLM.
- [Anthropic: effective harnesses for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents): provides context for persistent progress and verification across context windows. LocalLM already implements parts of that design, including durable state and compaction.
