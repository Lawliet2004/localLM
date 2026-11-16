# LocalLM inference and response improvement plan

Implementation follow-up: application repairs are now recorded in [the fix results](INFERENCE_AND_RESPONSE_FIX_RESULTS_2026-11-08.md). This document retains the original audit/plan as historical evidence.

Status: **planned, not implemented**. Based on the [8 November audit](/home/papan/Desktop/Projects/localLM/docs/FULL_CODEBASE_AUDIT_2026-11-08.md), current working tree and isolated reproductions. This plan covers all 16 open correctness findings, context-retention work Q01, and experience work U01–U03.

The first delivery must improve the conversation people actually read: tool identity, honest status, compact activity, useful command output and visible file changes. Backend P1 fixes proceed alongside that work. A cosmetic CSS pass alone will leave missing operations, wrong labels and broken generation behavior underneath it.

## Desired response experience

The default response should have a calm, readable final answer and a concise chronological trail showing how the application reached it. Details are available when inspected. The current action, an error or a required decision must remain apparent.

| Content | Default display | On inspection |
| --- | --- | --- |
| Assistant update | Short prose in the turn, visually subordinate to the answer | Normal selectable/copyable text; no repeated assistant chrome |
| Exposed reasoning | One quiet row per recorded reasoning segment, measured time only when available | Shared Markdown renderer; lazy content; user expansion survives streaming/reload |
| Read/search exploration | Group adjacent successful operations: “Read 6 files · searched 2 patterns” | Actual paths, patterns and per-call results; clipped output is a disclosure, not failure |
| Command | Command or human-readable title, exact lifecycle and elapsed time | Existing ANSI stdout/stderr renderer; bounded view; full output/artifact affordance |
| Persistent process | Started/Running/Exited/Stopped/Unknown and a stable process identity | Cursor-backed output and owned Stop; subsequent polls update that process |
| Web work | Query or page title, retrieving/complete/failed state, known source count | Source titles/domains, exact evidence IDs and errors; raw JSON secondary |
| File changes | Recorded paths and create/edit/delete/move state | Shared diff view with before/after identity; no fabricated net counts |
| MCP/generic tool | Human tool name and a concise result/outcome | Validated arguments, structured result, artifact and server identity |
| Approval | An inline request attached to its owning action/conversation | Command/path, workspace, effect and relevant diff; raw arguments expandable |
| Final answer | Existing appearance preferences, clear typography, strong opening outcome | Code, tables, math, citations and copy behavior from shared primitives |
| Turn result | Delivered/partial/stopped/failed and actual verification state | Exact checks, unfinished tasks, unknown outcomes and recovery options |

Successful settled details should normally collapse unless the user deliberately opened them. Errors, approvals and uncertain outcomes remain prominent. A completed tool request must not imply that the underlying process exited or that a task was verified.

Use the existing category/status token system, MarkdownView, CodeBlock, DiffView, highlight cache and ANSI parser. Preserve keyboard operation, reduced motion, scroll-up intent and exact copied source. Raw protocol details belong in inspection views.

**Product constraint:** presentation remains Settings-only. Do not add an Auto/Text/Visual selector, equivalent response-mode control, or per-conversation presentation editing to the homepage/composer. The proposed UI can improve ordinary Markdown responses without an extra model-formatting call.

## Delivery packages and dependencies

These are reviewable implementation units, not calendar estimates. Packages W01–W03 deliver visible frontend improvements while W04–W07 repair generation and tool behavior. Their order within that batch can follow available engineering capacity; urgent intent/truncation fixes should not wait for visual polish.

| Package | Work | Findings | Dependencies / exit evidence |
| --- | --- | --- | --- |
| W00 | Restore a trustworthy build baseline | B01, timing-test reliability | Typecheck passes; preserve initial cancellation failure and replace fragile synchronization |
| W01 | Normalize transcript operations and repair live identity | F01, F02, F03 | W00; repeated IDs, partial journals and every native alias represented correctly |
| W02 | Ship compact thinking/tool/command/web renderers | F06, U01, U02 | W01; a complete scripted turn is readable during streaming and after replay |
| W03 | Make file review and evidence links complete | F04, F05, B10 | W01; real native patch/read results and artifact-backed source metadata render correctly |
| W04 | Preserve provider reasoning and child final-output semantics | B02, B07, B08 | W00; captured parent/child requests and reopen history conform to each protocol |
| W05 | Separate displayed examples from control, and stop safely on loops | B03, B04 | W00; no explanatory syntax dispatches; heuristic stops never certify completion |
| W06 | Align tool contracts across discovery and workspace aliases | B05, B09; B10 parity review | W00; same permitted input/instructions through direct, discovery, PTC and child routes |
| W07 | Make verification follow actual mutations | B06 | W00; all execution aliases and relevant hooks invalidate stale checks |
| W08 | Wire production semantic checkpoints | Q01 | W04/W06/W07; constraints and completed-operation provenance survive compaction and reopen |
| W09 | Finish attention routing and measured UI performance | U03; W02 polish | W01/W02; background approvals and long-history interaction acceptance |
| W10 | Run comparative quality and native visual acceptance | All | The affected packages; retained task outcomes, traces and native frame sequences |

**First usable milestone:** W00 + W01 + a narrow W02/W03 slice covering reasoning → read/search → command → real file diff → final answer → reload. This is the first frontend delivery, not a reward deferred until every backend improvement is finished. Backend fixes W04/W05 should enter the same initial repair batch.

## W00 — Build baseline and regression evidence

1. Replace the incompatible `replaceAll` call with an ES2020-compatible implementation. Keep the existing human-readable activity labels and uncommitted response styling.
2. Convert the relevant audit diagnostics into production regression tests that assert **correct** behavior. Keep the diagnostic evidence separately; its intentionally defective assertions must not become a green release gate.
3. Repair the cancellation fixture's synchronization: wait for observed dispatch, issue cancellation, observe settlement, and use a generous bounded liveness timeout. Do not make application cancellation slower or remove the assertion merely to get a pass.
4. Capture provider payload fixtures and compact turn journals with secrets removed. Include real current native result shapes rather than hand-maintained lookalikes where possible.

Exit: typecheck, Rust check and existing frontend/library suites pass, with ignored/network acceptance clearly reported. No changes to the removed composer presentation controls.

## W01 — One transcript projection and stable operation identity

Introduce a pure, incremental projection between stored/live events and React. This is a frontend view model derived from the existing journal and `locallm.tool-envelope/1`, not a competing result envelope.

The projection should preserve:

- Conversation/run/step/call identity, parent operation, journal sequence and durable message ID.
- Requested and resolved tool names, known family/category and original arguments.
- Separate call-delivery, process-lifecycle, side-effect and verification states.
- Output clipping/artifact metadata independently of execution success.
- Per-file changes, evidence references and generic structured result metadata.

Use run + step + call identity for transient output keys. A persistent process gets its own conversation/workspace-owned identity. Legacy records need deterministic fallback keys and an explicit unmatched-record merge, not silent omission.

`ActivityTimeline`, `WorkSummary`, source collection and `ToolMessage` should consume this same projection. Centralize native alias/category handling; do not keep separate regexes that disagree about `file_write`, `apply_patch`, `search` and `visit`. Preserve safe generic behavior for third-party tools; a tool name alone never grants native trust or permissions.

Required regressions:

- Two `textcall_0` calls in different steps keep distinct commands and output.
- Reused IDs across runs do not cross-match; duplicate event delivery does not duplicate rows.
- Mixed journal/message-only calls all remain visible in chronological order.
- Deferred invocation displays the resolved operation while retaining requested-wrapper traceability.
- Every first-party registry alias has an intended category; mutation rows never enter a read-only exploration group.
- Live and reopened projections have the same operations, outcomes and file/source counts.

## W02 — Render the work clearly

Build focused renderers on the projection, keeping the existing shared content primitives. Prefer a small renderer registry to growing `ToolMessage`/`CommandRunCard` conditionals.

Implement explicit expansion ownership: an automatically opened live row can collapse on successful settlement; a user-opened row remains open. Failed, denied, awaiting and outcome-unknown rows must have visible labels and appropriate default expansion. A `partial` envelope with no execution error means bounded content, not automatic failure or exceptional attention.

Commands should show the actual command, workspace and known lifecycle. Parse process `output[]` frames into channels. Separate a successful launch from process exit, and separate exit 0 from a recognized passing test suite. Reconcile polls into the existing process record. A button labeled Stop command must target an owned process; a turn-wide cancel action must be labeled Stop response.

Web renderers must handle native web tools, AREX `search`/`visit` and generic MCP results. Show known queries/pages and source counts, and make no invented freshness or completion claims. Keep JSON/parameters secondary. The source list, inline citations and evidence inspector must agree after artifact loading.

Reasoning uses provider-exposed content only. Keep its state in the turn projection and mount expensive Markdown only when useful for display. Preserve recorded durations and disclosure identity across live/completed/reloaded states.

Exit evidence: a scripted mixed turn has inspectable thinking, grouped exploration, readable command output, web evidence, a final answer and an honest failure/stop variant. No duplicate activity indicators or automatic jumps when the user scrolls up or opens details.

## W03 — Accurate files and sources

Normalize native `changes[]`, older `diff`, first-party file creation and bounded artifact-backed results into file-change records. Include create/edit/delete/move and destination paths. Keep operation diffs distinct from a final net diff; obtain the latter from appropriate recorded before/after content or Git evidence.

Use a shared turn-owned evidence hook/store. Artifact hydration must update inline citation links, research context, footer and web details together. Retain citation provenance and exact IDs; if IDs are ambiguous across results, expose that ambiguity rather than selecting an arbitrary last URL. Missing metadata must not erase already known sources.

Repair B10 through trusted native file-read result handling. Preserve path, file hash, line/navigation metadata and instructions in a small model-visible record; distinguish the result-artifact checksum from the file hash. Do not apply native result semantics to an unrelated MCP tool with a similar name.

Required acceptance:

- Dispatch an actual two-file native patch; show both files and their recorded diffs after live delivery and reopen.
- Create/append/overwrite/delete/move with current transaction results; report failed/rolled-back/unknown outcomes honestly.
- Bound both `file_read` and `workspace_read_file`; retain usable file identity, instruction metadata and artifact access.
- Load an artifact-only research result; resolve the same citation in answer, sources and inspector.
- Handle unavailable artifacts without hiding the answer or inventing output.

## W04 — Provider and child round semantics

Extract shared round interpretation for text, provider reasoning, native opaque context, calls, terminal status and final answer. Preserve protocol-specific metadata separately from the text shown in the UI.

For Chat Completions, model reasoning replay requirements must be explicit capability/protocol policy. Preserve received `reasoning_content` where required across live rounds, final messages, durable history rebuild, retry and provider-compatible child execution. Do not reconstruct native payloads from rendered Markdown or assume every endpoint accepts the same extra fields.

For children, keep intermediate commentary in events and final output in a separate value. Apply schema validation to the completed final answer. Reuse the parent's supported inline-thinking parsing policy for affected local templates. Canonicalize text fallback consistently when native output cannot represent its calls, or disable it for that explicit native protocol under W05.

Required fixture matrix:

| Protocol/path | Required proof |
| --- | --- |
| Compatible Chat Completions with reasoning | Next request contains exact required reasoning plus the correct calls/results; persists after reopen |
| Native Responses | Original item/call IDs and opaque context survive; canonical/native calls remain identical |
| Native Claude | Thinking signatures and tool-use/result ordering remain valid |
| Local inline-thinking template | Thinking is separated from answer; code examples remain intact |
| Child commentary → tool → JSON | Final JSON validates and returned result excludes intermediate prose |
| Malformed/incomplete stream | No unfinished call dispatches; durable outcomes remain explicit |

Use injected transports and scripted servers first. Real providers and actual local model builds remain separate acceptance; simulated HTTP success is not model-quality proof.

## W05 — Intent-safe fallback and truthful repetition recovery

Compatibility tool parsing should be opt-in for a known model/template or configured wire protocol, not opportunistic for every prose response. Parse genuine control spans; preserve normal fenced/inline/quoted examples. The fallback decision, markup stripping, partial-call preview and truncation recovery must share the same interpretation.

Test XML, JSON and function-tag examples in backtick/tilde fences, nested lists, blockquotes, inline code and unfinished fences. Include a valid opted-in fallback call as a positive control. Confirm that the dispatch and permission path remains the normal one for genuine calls.

For repetition, separate detection from completion. Preserve requested duplicate code/data; do not stop merely because a later file shares an earlier header. If a guard must terminate a generation, record why, retain partial text, reject unfinished calls and apply a bounded continuation policy. A heuristic stop cannot become a fabricated provider success.

Test the audit's two-template case, repeated table data, deliberate repeated prose, a real runaway loop, loop-plus-incomplete-tool output, and exhausted recovery. Final delivery must accurately say partial/interrupted when recovery cannot finish.

## W06 — Consistent tool budgets and applicable instructions

Resolve a discovery target under the authorized catalog and apply its actual input budget. The wrapper transport must accommodate legitimate target arguments plus its own bounded encoding overhead, while respecting the global per-round cap. Maintain identifier, schema-validation, recursion and memory-work limits.

Verify inputs around 32 KiB and 64 KiB, 1 MiB file content, escaped/non-ASCII data, large patch batches, invalid/unknown targets and aggregate call limits. Exercise direct calls, `tool_invoke`, PTC and child dispatch. Advertised limits and actual rejection messages must agree; a model should not need to discover a hidden smaller transport limit through failure.

Move applicable nested instruction discovery into one trusted workspace operation path. Deduplicate root instructions already present; attach only applicable descendant rules with provenance and bounds. Ensure ordinary/native/catalog reads and subsequent edits cannot silently differ. Preserve the instructions after result capture.

Exit: route-parity fixtures demonstrate the same accepted inputs and relevant project rules across each supported dispatch entry point, with unchanged authorization and no-follow workspace guarantees.

## W07 — Evidence follows mutations

Replace the verification name-list gap with authoritative tool effect metadata and an observed workspace revision. Cover native write aliases, local execution, PTC, child/workflow work and configured mutation-capable hooks. A command that may mutate should invalidate prior proof unless a trustworthy source-bound comparison establishes otherwise.

Keep these separate facts: response delivered; required work complete/partial; process terminal result; recognized test result; source freshness and selected coverage. A passing build is evidence about that command, not a claim about unperformed tests or visuals.

Tests must include edit → pass → `local_run_code` mutation; edit → pass → `local_execute_command` mutation; a post-check hook mutation; failed/unknown partial writes; a failed suite followed by an unrelated successful command; disabled verification tools; and an honest unverified final response. Preserve unfinished todos. W08 checkpoints must retain these actual states.

## W08 — Semantic continuity rather than archive-only memory

Wire the existing checkpoint concepts into production compaction. Retain original task/requirements, explicit constraints, important decisions and rejected approaches, changed paths, durable completed-operation IDs, evidence URLs/IDs, unresolved work, verification state and next action. Keep the verbatim archive as recovery and provenance.

Bound and validate checkpoint generation. A model-authored summary cannot invent completed operations, permissions or verified facts; derive those fields from durable records. If semantic summarization fails, preserve the last usable checkpoint and a truthful limitation. Do not execute tools while summarizing or add a second incompatible history format.

Handle whole-turn boundaries and the separate case of a single long active exchange. Maintain valid native tool adjacency and opaque-context requirements. Do not silently trim an incomplete action to fit.

Acceptance needs generation-level follow-up tasks, not just serialization tests: a requirement from 15 turns earlier remains answerable after repeated compaction and reopen; an earlier write is not repeated; citations and file identities resolve; pending tasks remain pending; changed instructions refresh correctly; and current user corrections override older summaries.

## W09 — Attention, accessibility and long sessions

Keep approval requests bound to their chat and action. Background tabs receive a clear attention indicator; the user can inspect that request without an unrelated modal taking over their current reading. Preserve approval ownership, expiry and cancellation semantics.

Measure native input/scroll/disclosure latency under a long transcript, 20,000 journal events, 1,000 tool rows, multi-round reasoning and two active conversations. Observe hidden-session update pressure and committed render work. Cache/reduce incrementally and lazy-mount details first. Add virtualization only if the measured profile warrants it, with durable disclosure/selection/scroll state.

Preserve the current terminal bounds (30 folded lines / 500-line pages), output following only at the bottom, keyboard-visible focus, semantic labels, text selection and exact copy. Screen-reader announcements should describe meaningful state changes, not every token.

Suggested performance acceptance targets, to validate on named reference hardware: p95 local input/disclosure response under 100 ms; no repeated >50 ms main-thread tasks during ordinary token delivery; no transcript jump after opening details or scrolling upward. These are proposed budgets, not measurements achieved by this audit.

## W10 — Demonstrate quality and visual acceptance

### Controlled harness comparison

Compare LocalLM before/after and OpenCode using the **same model endpoint/build**, quantization where applicable, context/output budgets, reasoning setting, temperature, workspace, task input and tool availability. Record differences that cannot be held equal. Use Claude Code as a product experience reference; different models are not a controlled harness-quality comparison.

Start with 24 tasks across ordinary answers/formatting (4), repository investigation and multi-file edits (6), commands/process/checks (4), grounded web responses (4), and long-context/failure recovery (6). Add the confirmed regressions as mandatory correctness cases. Run repeated samples for nondeterministic generation; report raw outcomes and sample sizes before making statistical claims.

Measure actual artifact/task success and unsupported completion claims, tool selection/argument validity, unnecessary retries/rereads, number of model rounds, retained requirements, citation correctness and final-answer readability. Separate preparation, first model token, first visible answer, provider generation, tool work, approval waiting and total task duration. Prompt-cache estimates must not become claimed provider cache hits.

Use recorded retrieval fixtures for deterministic checks and separately identify real network freshness tests. Preserve provider errors and app errors separately. Report partial/unverified tasks honestly; do not turn refusal, cancellation or intentional disabled capability into a benchmark success.

### Native response acceptance

Capture a synthetic native Tauri fixture sequence for: plain answer; long Markdown/code/table/math; multiple reasoning rounds; file exploration; native batch patch; streamed command; persistent process; web sources/citations; approval allow/deny/expiry; MCP form; failure; cancellation/unknown outcome; output clipping; compaction; failed-round continuation; and reopen/replay. Use both themes at the native minimum 760×560, 1280×840 and a larger window, plus 125–200% scaling and keyboard navigation.

Review reading hierarchy, space allocated to the transcript, truncation/wrapping, output inspection, source navigation, file review and attention states. Compare live-to-complete-to-replay frame sequences, not just one final screenshot. Use a permitted native fixture host; the earlier declined localhost browser preview must not be bypassed with another browser/proxy route.

The interactive proposal accompanying this plan uses synthetic content and DOM-checked states. It is a discussion target, not evidence that these native acceptance cases pass.

## Completion checklist

- [ ] All B01–B10 and F01–F06 regressions assert repaired behavior and pass.
- [ ] W02/W03 deliver the complete response path in the first frontend milestone.
- [ ] Q01 production compaction preserves useful requirements and durable operation provenance.
- [ ] U01–U03 have explicit default behavior, native acceptance and measured performance evidence.
- [ ] Rust/library/frontend/type checks pass; initial failures, skips and host restrictions remain explicit.
- [ ] Actual provider/local-runtime acceptance is distinguished from scripted fixtures.
- [ ] Model/task comparison and native captures support any claim of improved quality.
- [ ] `docs/RENDERING.md` matches the final renderer/projection contract.
- [ ] No removed homepage/composer response-mode control was restored.
- [ ] Existing user work is preserved; each implementation package has a reviewable diff and rollback path.

## Reference basis

OpenCode source was inspected at `ece56ca58419083275f2a2a81ac28d05f4a1b89b`, particularly its [typed timeline](https://github.com/anomalyco/opencode/blob/ece56ca58419083275f2a2a81ac28d05f4a1b89b/packages/session-ui/src/timeline/timeline-row.ts), [tool renderer registry](https://github.com/anomalyco/opencode/blob/ece56ca58419083275f2a2a81ac28d05f4a1b89b/packages/session-ui/src/tools/tool-renderer.tsx), and [shell follower](https://github.com/anomalyco/opencode/blob/ece56ca58419083275f2a2a81ac28d05f4a1b89b/packages/session-ui/src/tools/shell-output.ts). Claude Code's [documented desktop transcript behavior](https://code.claude.com/docs/en/desktop#switch-view-modes) informs progressive inspection and answer hierarchy. These are references for design and state handling, not proof that copying their libraries improves LocalLM's model output.
