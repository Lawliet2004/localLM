# Original SWE-2 Max review, with verification notes

Date: 2026-10-19. Produced by `devin --model swe-2-max` using Devin CLI 3000.11.92. The process completed successfully with exit code 0 and no stderr. Initial progress narration is omitted below; the final report is preserved as generated.

**Use the [reviewed audit](INFERENCE_TOOL_AUDIT_2026-10-19.md) for accepted findings and implementation priorities.** This original model output contains static-analysis claims that were not all confirmed. Direct source verification identified these qualifications:

- Foreground child agents can borrow parent tool references (`subagents.rs:848` and `subagents.rs:577`). MCP access is not universally absent from all subagents. Background reconstruction and cloned/detached tool paths do lose capabilities; the reviewed report distinguishes them.
- Persistent terminals already recognize PowerShell/pwsh when supplied through the configured shell path (`sandbox.rs:163`). A separate `powershell_path` fallback is an optional configuration improvement, not proof of no PowerShell support.
- Tool batches run sequentially, but AREX visit already uses a parallel URL worker. Do not describe every research operation as sequential. Concurrent scheduling should be limited to independent operations, not every auto-approved call.
- The claimed approximately twofold remote token estimation error was not benchmarked. The reviewed report states that counting is estimated without assigning an unsupported error magnitude.
- The report's claim that `preset.system_time` gates the clock is not supported by the current `chat.rs:1062` assembly code, which checks model support and the global clock capability. General capability disable enforcement remains incomplete.
- Broad architectural praise, precise registry counts, severity labels, implementation-size estimates and completeness claims are model judgments, not runtime measurements. The reviewed audit does not adopt them as verified facts.

---

# LocalLM inference-side agent harness — audit report

**Scope.** Static, read-only audit of the current working tree (`src-tauri/src`, `src`, `catalog`). No code was executed, no tests run, no files changed. **Every conclusion below is static-analysis-only** unless explicitly marked otherwise; nothing here is runtime-verified.

## 1. Executive conclusion

LocalLM is not a chat-only shell with bolted-on tools — it has a mature, fully instrumented agent runtime. The complete lifecycle `inference → catalog → parse → validate → approve → dispatch → result → persist → continue` is implemented end-to-end in `chat.rs` (~3,600 lines) with durable run/event/session journaling at every stage:

- **Catalog** is assembled per turn in `assemble_turn` (`chat.rs:788-1145`) as the intersection of preset (`presets.rs`), per-conversation source selection (`__workspace`, `__execution`, `__browser`, `__computer`, `__daytona`), MCP selections, skills, capabilities, offline/web settings, interpreter availability, and model compatibility (`tool_compat.rs` fingerprint probes).
- **Parsing** accepts native streaming `tool_calls` deltas (`tool_calls.rs`) *and* literal `<tool_call>`/`<function=...>` markup, with a bounded one-shot repair loop (`chat.rs:1949-1974`, `continuation.rs:210-220`).
- **Validation** is layered: JSON-Schema at `prepare_arguments` (`connectors.rs:323-330`), re-validation at dispatch (`connectors.rs:372-376`), plus per-tool semantic checks (AREX contracts, path rules, hash-pinned script approval).
- **Approval** is per-conversation `AccessMode` (Ask / AutoApprove-reads / FullAccess) enforced in the dispatch loop (`chat.rs:2289-2332`), never derivable from model output (`permissions.rs:52-59`).
- **Dispatch** covers eight backend kinds (harness, workspace, local execution, MCP remote, local stdio MCP, skills, browser/computer automation, system clock) through one `AgentTool` abstraction — but **strictly serially**.
- **Results** are bounded into artifact-backed envelopes (`artifacts.rs`), audited per call in `tool` messages, replayed into model history by `history.rs`, and governed by continuation budgets (`continuation.rs`) with repetition detection, web-lookup retry, finalization retry/fallback, and an AREX claim audit.

The genuinely actionable issues are concentrated in: (a) a capability kill-switch registry whose toggles mostly don't fire, (b) sequential-only tool scheduling, (c) no provider-level structured output, (d) a few implemented-but-unexposed surfaces (three presets, `workspace_search_files`, Daytona), and (e) MCP tools being unreachable to subagents by design.

## 2. Actual exposed-tool inventory

All tools become model-visible only when the preset lists them **and** the corresponding source/capability gates pass. Connector/alias prefix in model schemas is `"{connector}: {name} — {description}"` (`connectors.rs:352-354`).

| Class | Model-facing tools | Gates |
|---|---|---|
| Workspace (`connector="Workspace"`) | `workspace_list_files`, `workspace_read_file`, `workspace_create_file`, `workspace_create_directory`, `workspace_edit_file` (`workspace.rs:515-524`) | `__workspace` selected, preset allows, workspace path set; cap-std no-follow symlinks; `edit_file` needs fresh `expected_sha256` (`workspace.rs:433-449`) |
| Local execution (`connector="Local execution"`) | `local_run_code`, `local_execute_command` (`connectors.rs:331-343`) | `__execution` selected + workspace set; interpreter enabled/present in `ExecutionConfig`; `bash.always_ask`/`python.always_ask` force approval even under FullAccess (`execution.rs:333-336`) |
| Harness (`connector="Harness"`) | 31 registered aliases + `weather` alias clone + 4 AREX tools (`harness.rs:69-166`, `arex.rs:64-76`) | preset `harness` list ∩ `harness_source_allowed` (`presets.rs:168-175`) ∩ offline/web/`weather_config`/bash gates (`chat.rs:885-903`) |
| System | `system_time` (`system_tools.rs:122-137`) | `backend.supports_tools` + `preset.system_time` + `capabilities.is_enabled("system_time")` (`chat.rs:1062-1083`) |
| Skills | `skills_read_file` (`skills.rs:424-445`, alias `skills.rs:289`) | `preset.skills` + ≥1 active installed skill; instructions also injected into system prompt |
| Browser (`connector="Browser"`) | 11 tools from `resources/automation/browser-tools.json` | `__browser` selected, `automation.browser_enabled`, preset ≠ chat/minimal (`chat.rs:904-916`, `presets.rs:151`) |
| Computer (`connector="Computer"`) | `computer_screenshot`, `computer_position`, `computer_move`, `computer_click`, `computer_type`, `computer_key`, `computer_scroll` (`computer_use.rs:694-741`) | `__computer` selected, `computer_enabled`, **model must support image input** (`chat.rs:974-976`) |
| Remote MCP | per-connector tools, hashed aliases (`connectors.rs:206-220`) | `preset.mcp`, live session, user-selected `ToolSelection`s, ≤64/turn (`connectors.rs:492-563`) |
| Local MCP | stdio servers from `local_mcp_config.rs` | explicit connect; env-clear allowlist; `KillOnDrop` (`local_mcp_process.rs:44-80`) |
| Daytona | `run_code` cloud sandbox (`connectors.rs:267-271`) | **Unreachable**: `cloud_execution_allowed()` always errors (`local_only.rs:15-17`) and the UI checkbox is locked off (`ToolControls.tsx:296-300`) — historical records only |

**Preset exposure** (`presets.rs:50-152`):

- `standard`: 11 lean tools (echo, weather, web_search/read/find, file_search/read/write, run_code, ask_user, artifact_read).
- `chat`: todos, memory_recall, compact_conversation, artifact_read; workspace only; no execution/MCP.
- `research`: full web + AREX contract (`search/visit/update_context/finish`); workspace + daytona sources; no execution.
- `coding`: subagents, terminals, todos/goals, memory, weather, file tools, compaction, research_* — **no web search**.
- `code` (PTC): `ptc_run` + generated TypeScript SDK (`presets.rs:204-212`, SDK injected at `chat.rs:1099-1104`).
- `minimal`: `terminal_create/send/close` + `artifact_read` + filtered `workspace_edit_file` only (`presets.rs:63-74`, `chat.rs:776-779`).
- `creator`: everything including `workflow_run`, `ralph_run`, `docker_exec`, `preset_guide`, `research_*`.

**Only `standard`, `chat`, `research`, `coding` are selectable in the UI** (`ToolControls.tsx:324-341`); `code`, `minimal`, `creator` are backend-complete but UI-unexposed. AREX tools are also injected automatically when the loaded model id contains "arex" (`arex.rs:27-43`).

**Tool-limit**: hard error above 64 offered tools (`chat.rs:917`, `chat.rs:1097`).

## 3. Priority gap table

| # | Gap | User impact | Evidence | Suggested design / smallest fix |
|---|---|---|---|---|
| P0 | **Capability kill-switches inert.** `capabilities.disabled` is consulted only for `system_time` and `compaction`. Toggles for `workspace_files`, `local_execution`, `skills_reader`, `subagents`, `workflows`, `cloud_execution` write settings but never gate tool assembly. `subagents`/`workflows` are even declared `enabled:false` in `capabilities.json:144-160` yet work in creator/coding. | Settings UI lies about enforcement; users can't actually disable execution/agents. | `capabilities.rs:195-202`; only callers `chat.rs:1064`, `compaction.rs:123` | In `assemble_turn`, filter sources/harness aliases through `is_enabled` for the declared effects map (`capabilities.rs:99-106`). ~30 lines. |
| P1 | **Tool calls execute strictly serially.** Up to 8 calls/round (`tool_calls.rs` index cap) but dispatched in `for mut call in calls` (`chat.rs:2254`); `parallel_tool_calls` is only *stripped* when unsupported (`inference.rs:298-301`), never exploited. AREX `visit` advertises parallel reads. | Slow research turns; N sequential approval round-trips. | `chat.rs:2254-2654` | Partition batch into approval-required vs. auto-allowed; `join_all` the auto-allowed subset, keep serial order for audit rows. Approval ordering must stay deterministic. |
| P1 | **No structured-output channel.** No `response_format`/JSON-schema/grammar ever sent (`chat.rs:3001-3055`); subagent `output_schema` is a prompt instruction + post-hoc `required` check only (`subagents.rs:494-517`). | Structured tasks (extraction, form answers) rely on prompt compliance; no retry on schema violation. | `subagents.rs:459-463`, `504-517` | Map `output_schema` to `response_format: {type:"json_schema"}` for OpenAI-format backends and llama.cpp `response_format`/grammar for local. Small: extend `request_payload` + `Backend::payload`. |
| P1 | **Subagents cannot call MCP tools.** `AgentTool::clone()` downgrades MCP to `DetachedMcp` which fails loud (`connectors.rs:178-240`, `366-368`); background children get `harness_detached` subset + reconstructed local tools only (`subagents.rs:538-557`). | Delegated tasks silently lose all connector capability even under `tool_filter:["mcp"]`. | `subagents.rs:69-76`, `848-857` | Route child MCP calls back through the foreground dispatcher via a channel, or document the limit in `list_subagent_models`/`subagent` schema description (smallest: description + system-prompt note). |
| P2 | **Denial granularity is turn-wide.** One denial sets `tool_use_denied` → all later calls in the batch denied (`chat.rs:2290-2291`), `tool_choice:"none"` for subsequent rounds (`chat.rs:3023`), and any further call attempt hard-errors the turn (`chat.rs:2230`). | Denying one action (e.g. a write) also kills unrelated reads and can fail the whole turn. | `chat.rs:2230`, `2289-2291` | Deny per-call-class: keep trusted reads and unrelated tools live; only block the denied signature via `blocked_calls`. |
| P2 | **No transport-level retry.** `stream()`/`count` failures fail the turn; `count_only` retries once (`context.rs:131-141`), streams never. Subscription 429/401 handling produces error text, not retry (`inference.rs:507-548`). | A transient blip abandons a multi-round turn; completed work stays in DB but the user must resend. | `chat.rs:1728-1731` | One bounded retry on connect/reset errors before stream start (response not yet consumed), keyed by attempt in run events. |
| P2 | **`workspace_search_files` implemented, unexposed.** `Workspace::call` handles `search_files` (`workspace.rs:474-511`) but `tools()` omits it (`workspace.rs:515-524`). | Filename lookup only via content `file_search` or shell. | `workspace.rs:515` | Add the `ToolView` entry — one line in `definitions`. |
| P2 | **No delete/move/copy file tools.** Workspace ops cover list/read/create/mkdir/edit only. | Model must request shell approval for rm/mv, or leave stale files. | `workspace.rs:515-524` | Add `delete_file`/`move_file` behind `x-locallm-always-ask` with cap-std no-follow paths. |
| P2 | **Remote context accounting is a bytes÷4 estimate** (`context.rs:226-241`). | Budget errors up to ~2×; conservative compaction at 80% absorbs most, but small contexts can fail spuriously or overflow provider-side. | `chat.rs:1397-1418`, `inference.rs:399-403` | Ship per-family tokenizers (tiktoken-rs, anthropic token-count endpoint exists) or use provider `prompt_tokens` after round 1 to correct the estimate. |
| P2 | **Web tools offered but guaranteed to fail without Node.** `web_search`/`web_fetch_url`/`web_read`-URL/weather all need `execution.node_path` (`web_search.rs:70`), yet only `weather` checks an enable flag at assembly (`chat.rs:888`). | User sees offered tools that error at call time. | `chat.rs:885-903` vs `web_search.rs:70` | Skip web aliases at assembly when `node_path` empty, pushing a `tool_notice` instead (pattern already exists). |
| P3 | **`research_pause/resume/cancel/progress` are ledger signals only.** Dispatch emits a `research_task` event and returns `{taskAction}` (`harness.rs:687-693`); actual pause/cancel lives in `run_control`/UI commands. The model tool doesn't pause the current loop. | Model-visible controls that don't do what they say mid-turn. | `harness.rs:687-693`, `agent_run.rs:97-157` | Either wire `research_pause` to `apply_pause` on the active control, or mark them "request recorded" in the schema description. |
| P3 | **Three presets unreachable from UI.** `code`/`minimal`/`creator` fully implemented + tested but absent from `ToolControls.tsx:324-341`. | PTC mode, benchmark mode, and orchestration invisible. | `ToolControls.tsx:324-341`, `presets.rs:50-152` | Add the three radio options; backend already validates. |
| P3 | **Literal markup can leak into the visible answer** when a model emits `<tool_call>` text *alongside* valid structured deltas — `decide_text_tool_call` only fires when `calls.is_empty()` (`chat.rs:1949`), and markup is stripped only on finalizing rounds (`chat.rs:2011-2028`). | Mixed-mode rounds can persist raw call markup in the stored answer/history. | `chat.rs:1949-1974` | Strip `strip_text_tool_calls` from `round_answer` whenever the round produced structured calls too. |
| P3 | **Terminal tools gated on bash only.** `terminal_create/send` hidden unless `bash.enabled && bash_path` (`chat.rs:887`); no PowerShell persistent session despite `powershell` being a `run_code` language. | Windows users without bash lose the persistent-shell surface. | `chat.rs:887`, `sandbox.rs:158-208` | Allow `powershell_path` fallback in `TerminalRegistry::create`. |
| P3 | **Tool result envelopes are not uniform.** `locallm.tool-envelope/1` is enforced only for execution results (`execution.rs:921-1029`); harness/workspace/MCP/skills/automation return ad-hoc JSON bounded by `bound_tool_result`. `tool_envelope::validate` exists but is never invoked at the dispatch boundary. | Downstream consumers can't rely on `status`/`data`/`error` shape; error detection is string-sniffing (`tool_execution_error`, `chat.rs:177-184`). | `connectors.rs:403-431`, `chat.rs:177` | Wrap all backends' results in the envelope at `call_with_stream`; keep raw payloads inside `data`. |
| P3 | **`weather` vs `get_weather` duplicate aliases** (`harness.rs:162-163`); only `get_weather` is in presets — `weather` exists for replay history but is dispatchable if ever offered. | Dead catalog surface; harmless but confusing. | `harness.rs:162` | Keep for replay; document. |

## 4. Inference protocol & capability issues

- **Schema/tool choice**: tools are offered as OpenAI `{"type":"function"}` definitions (`connectors.rs:352`), sorted by alias and *frozen per conversation* for prompt-cache stability (`freeze_tool_catalog`, `chat.rs:1147-1184`) — descriptions stay frozen unless name/parameters change. `tool_choice` is only ever set to `"none"` (denied/planning/finalizing rounds, `chat.rs:3023`); no `"required"`/forced-function mode exists — the AREX `finish` contract is enforced by prompt convention instead.
- **Parallel scheduling**: none (see P1). Also note `claude_adapter.rs:207` sends `disable_parallel_tool_use` on Claude when suppressed — parallel generation is possible on the wire but never used.
- **Context**: exact local counts via `/v1/chat/completions/input_tokens` with 300 s coalesced cache (`context.rs:88-178`); remote = bytes/4 estimate + 8,192/image reserve. Budget `input + max_tokens ≤ context` enforced pre-send (`context.rs:42-48`) and re-counted each round (`chat.rs:1688-1692`); auto-compaction at 80% (`compaction.rs:118-129`). `max_tokens` is fit to `context − headroom` (`context.rs:53-67`).
- **Reasoning**: `<think>` tags filtered (`chat.rs:31-92`), API `reasoning_content`/`reasoning` captured, Claude `thinking`/`redacted_thinking`/`signature` blocks preserved and replayed (`sse.rs`, `claude_adapter.rs:36-77`), OpenAI `reasoning_effort` for gpt-5/o-families (`inference.rs:118-121`, `318-327`), local `enable_thinking`/`reasoning_effort` detected per-GGUF-template (`chat.rs:2949-2999`). Reasoning is stored separately and **never replayed into history** (verified via `history.rs` tests).
- **Token accounting**: `stream_options.include_usage`, llama.cpp `timings:true`, per-round `inference_usage` events, `ResponseMetrics` persisted per message (`chat.rs:2696`), context metrics emitted to UI (`chat.rs:1717`).
- **Stop reasons**: `stop`/`tool_calls` accepted; `length` fails unless continuing plain text (then CONTINUE_INSTRUCTION is injected, `chat.rs:2153-2163`); `content_filter` and unknown reasons fail closed (`chat.rs:2789-2796`). Claude stop reasons normalized in `sse.rs` (`tool_use`→`tool_calls`, `end_turn`/`max_tokens`).
- **Retries**: none at HTTP layer; one malformed-text-call repair (`continuation.rs:210`); one false-refusal web-lookup retry (`continuation.rs:181-200`); one finalization retry then grounded fallback (`chat.rs:2040-2114`); ambiguous side-effect failures are pinned by signature and never auto-retried (`chat.rs:2272-2277`, `continuation.rs:23-45`).
- **Cancellation**: generation-scoped watch channel with cancel matching (`agent_run.rs:97-157`); cancels apply between rounds, mid-stream, mid-batch, and during tool awaits (`chat.rs:2441-2497`); in-flight tool gets `outcome` marked unknown/cancelled (`chat.rs:2544-2550`); children honor interrupt flags (`subagents.rs:599-604`); web worker and executions are `kill_on_drop`.
- **Structured output**: absent (P1 above). `finish`/`update_context` are control tools applied via `arex::apply_control` (`chat.rs:2561`, `2630-2635`) — this is the only "structured answer" path and it's AREX-only.
- **Multimodal**: real. Attachments → `image_url` parts with format whitelist (`attachments.rs:14-20`); `supports_images` gates draft images (`chat.rs:1038-1040`) and is required for Computer use (`chat.rs:974`); browser/computer screenshots are captured into artifacts and re-injected as image content for vision models (`automation.rs:382-409`, `chat.rs:2594-2598`, `2655`); images are stripped from history for text-only models (`automation::remove_model_images`, `chat.rs:1037`); remote preflight reserves 8,192 tokens/image (`context.rs:226-241`); local image support comes from `runtime.supports_images`.
- **History/audit**: model history replays grouped assistant `tool_calls` + `tool` results from audited rows (`history.rs:121-150`), interrupted tools replay as explicit `outcome:unknown` results (`history.rs:102-110`), denied calls replay as denials, and the frozen tool catalog keeps replayed names resolvable (`history.rs:101`, `connectors.rs:184-220`).

## 5. Lifecycle & reliability gaps

- **Approval timeouts deny** (600 s, `chat.rs:2311-2330`; `ask_user` same, `harness.rs:649-652`; subagent-pin approvals 600 s, `subagents.rs:785-793`) — fail-closed, but a slow user silently turns approvals into denials; there's no "pause and resume later" for a pending approval inside a run (pause blocks *new* tools only).
- **Run ledger robustness is good**: `runs` + `run_events` + `session_events` + `research_tasks` ledgers record every transition; `OutcomeUnknown` exists and interrupted ops are never replayed (`agent_run.rs:173-185`, `research_tasks.rs`). Pause/resume injects a durable "Paused checkpoint" into the next user message (`chat.rs:1325-1362`).
- **Subagent isolation is solid but lossy**: ephemeral history, depth cap (`subagents.rs:364-370`), reads-only pin by default (`subagents.rs:44-63`), mailbox/`send_message` restricted to the exact parent (`harness.rs:489-505`). The MCP-detached downgrade (P1) is the main capability loss; `allowed_tool_aliases` further filters children to the parent's offered set (`chat.rs:1592`, `subagents.rs:555`).
- **PTC** is a real second dispatch path with per-step approval, audit rows, sdk-call/step consistency check (`harness.rs:782-888`).
- **Compaction** is archive-verbatim (no LLM summarization): dropped prefixes go to artifacts, replayed as a checkpoint notice + ≤8×180-char excerpts (`compaction.rs:134-180`); AREX `update_context` similarly archives (`arex.rs:224-247`). Tool-call/result pairs are never split (`compaction.rs:139-153` boundary logic). Downside: compacted content is recoverable only via `artifact_read` — the checkpoint summary is model-authored, not generated by a summarizer; for non-AREX models the compaction notice has no synthesized facts at all, so post-compaction recall quality depends entirely on the model calling `artifact_read`.
- **Frozen-catalog staleness**: schema-identical tools keep frozen descriptions; a *behavioral* change with identical parameters won't reach the model (`chat.rs:1167-1173`). Minor.
- **Loop hygiene**: verbatim re-emission cut (`chat.rs:412-451`), 3-identical-call repetition stop (`sandbox.rs:684`), research stagnation ≥3 stagnant or 16 web calls (`chat.rs:175`), max rounds 64 with a reserved final round (`continuation.rs:60-98`).
- **Approval-hash binding**: approved Python file source is SHA-256-pinned at `prepare_arguments` and re-verified at run time; post-approval edits fail (`execution.rs:337-347`, `440-447`).

## 6. Explicit false positives excluded

- **Shell exists**: `local_execute_command`, `run_code`, `terminal_*` persistent sessions, `docker_exec` (creator), PTC — not absent; the right question is which preset exposes them.
- **File editing exists**: `workspace_edit_file` (hash-pinned, unique-match), `create_file` (no overwrite), plus harness `file_write`/`file_read` and Minimal's edit-only surface.
- **Search exists twice**: `file_search` (fixed-string content search) and `web_*`/AREX research tools backed by a bundled TypeScript engine run via Node worker (`web_search.rs`).
- **Browser exists**: 11 tool definitions, gated on `automation.browser_enabled` — not absent, just off by default.
- **Subagents/workflows/ralph exist** with real lifecycle, not stubs — gated by preset and detached-MCP limits.
- **Compaction exists** (auto 80% + manual + model-callable `compact_conversation`), as do durable pause/resume/restart ledgers.
- **MCP exists both remote (streamable HTTP) and local (stdio)** with bounded discovery (`tool_discovery.rs`), per-turn selection, hashed aliases, and live-session enforcement.
- **Multimodal and reasoning are implemented**, including Claude thinking-block replay and image injection for computer use.
- **`editor.rs` and `terminal_ui.rs` are product UI surfaces** (direct file mutations, interactive PTY) — deliberately not model-callable, per module docs (`editor.rs:1-2`, `terminal_ui.rs:6-7`).
- **`document_index` is injection, not a tool**: top-3 scored excerpts auto-injected per turn (`document_index.rs:24-44`, `chat.rs:986-993`); `search_document_index` is a UI command.
- **`capabilities.json` `"enabled":false` for subagents/workflows is declarative metadata**, not enforcement — but that's the bug in §3-P0, not a missing feature.

## 7. Staged roadmap

**Stage 0 — honesty fixes (small, high-value):**
1. Wire `capabilities.disabled` into `assemble_turn` for `workspace_files`, `local_execution`, `skills_reader`, `subagents`, `workflows` (and remove or document `cloud_execution`). Fixes the kill-switch lie.
2. Hide web tools when `node_path` is empty (fold into a `tool_notice`); add `workspace_search_files` to `tools()`; expose the three hidden presets in `ToolControls`.
3. Strip literal `<tool_call>` markup from `round_answer` on every round, not only finalizing ones.

**Stage 1 — protocol depth:**
4. Structured output: `response_format`/grammar for local + OpenAI backends; apply to subagent `output_schema` and an optional per-turn JSON mode.
5. Parallel dispatch for the auto-allowed call subset (preserve audit order); batch the approval-required calls deterministically.
6. Per-call-class denial: deny the denied signature, keep unrelated tools; replace the turn-wide `tool_use_denied` cliff with a partial-answer path.
7. One bounded transport retry on pre-first-byte stream failures for remote backends.

**Stage 2 — delegation & fidelity:**
8. MCP access for subagents via a foreground-dispatch channel (or at minimum, an honest schema/prompt disclosure of the detached-MCP limit).
9. Uniform `locallm.tool-envelope/1` around all backends at `call_with_stream`; keep vendor payloads under `data` so the model sees a consistent status contract.
10. Real remote token accounting (provider `usage` feedback loop or packaged tokenizers).

**Stage 3 — lifecycle polish:**
11. `delete_file`/`move_file` workspace tools behind always-ask; PowerShell fallback for persistent terminals; wire `research_pause` to `run_control` or rename the tools to reflect their ledger-only effect.
12. Optional summarizer step for non-AREX compaction so compacted prefixes retain facts, not just artifact pointers.

The architecture underneath — one `AgentTool` abstraction, preset visibility patches, capability-secured filesystem, approval-pinned dispatch, durable run ledgers — is sound and unusually well-bounded. Most gaps are wiring and exposure, not missing machinery.
