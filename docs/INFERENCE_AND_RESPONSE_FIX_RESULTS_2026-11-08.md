# Inference and response fixes — 8 November 2026

Implementation follow-up to the [audit](FULL_CODEBASE_AUDIT_2026-11-08.md) and [repair plan](INFERENCE_AND_RESPONSE_FIX_PLAN_2026-11-08.md). Application code has now changed. The original audit and diagnostic fixtures remain historical evidence; their assertions intentionally reproduce the old defects and are not the acceptance tests for these repairs.

The 16 concrete findings have corresponding repairs. Context handoff, disclosure behavior, background approvals, and the response layout were also updated. Existing uncommitted work was retained. No response-mode selector was added to the homepage or composer.

## Correctness changes

| Finding | Repair | Regression / verification |
| --- | --- | --- |
| B01 | Replaced `replaceAll` with an ES2020-compatible regular expression. | TypeScript check and embedded application build. |
| B02 | The shared stream adapter retains the exact compatible reasoning fields actually received. They enter canonical assistant replay and the existing durable native-message history path. Claude/Responses opaque content retains its own protocol. | `audit_fixes::compatible_reasoning_survives_round_replay`; existing native history suites. |
| B03 | Literal control parsing is restricted to the owned local template. Standalone control spans are distinguished from fenced, indented, inline, blockquoted and quoted examples. Genuine file-write argument bytes, including embedded backticks and fences, are preserved. Detection, parsing and stripping share this interpretation. | `quoted_control_regressions`, `audit_fixes::quoted_examples_stay_inert`, parent and child stream suites. |
| B04 | The bounded prose loop guard requires three consecutive copies. Code, structured data, tables and explicit repetition requests are protected. Guard termination returns a visible partial/error outcome and never invents a successful provider stop. | `repetition_cut_finds_only_verbatim_reemission`, `audit_fixes_repetition`; data/template controls. |
| B05 | Discovery transport has a bounded allowance for target JSON plus envelope overhead. Resolution checks the authorized target's actual argument limit. Existing aggregate round limits and target schema checks remain in effect. | Direct/discovered 40 kB write acceptance and oversized-target rejection in `audit_fixes`; discovery suite. |
| B06 | Verification names derive from the typed dispatch backend. Local/cloud executable code invalidates prior evidence. Both pre- and post-tool shell hooks invalidate evidence, including hooks with empty stdout. This is conservative because arbitrary shell effects cannot be certified from a note. | Verification progress suite, local alias regression, hook suites. |
| B07 | Children collect visible output per round, filter leading compatible inline thinking and validate only the final round. Commentary stays in protocol history. Received output, including discarded reasoning, remains bounded. | `audit_fixes_children::compatible_child_separates_inline_thinking_from_answer`, `child_validates_only_the_final_round`. |
| B08 | Native providers use structured calls; literal examples remain text. A shared canonicalization helper removes conflicting native payloads when explicitly using literal compatibility calls. | Native Responses child replay control and existing native child suites. |
| B09 | Nested project instructions attach at the trusted workspace boundary for native and harness aliases, including result paths from transactions. | `audit_fixes::workspace_aliases_share_nested_rules`. |
| B10 | Bounded first-party harness reads preserve file SHA-256 and path metadata. Applicable rules are retained when they fit; otherwise an explicit notice directs the model to the full artifact before editing. Arbitrary MCP payloads do not receive native file semantics. | `audit_fixes::bounded_harness_read_preserves_file_identity`; artifact suite. |
| F01 | Live operation identity includes run, step and call. Stream buffers and settlement use that identity; new durable audits retain run/step IDs. Legacy records also join by exact result message IDs or bounded fallback matching. | Production App regression with reused `textcall_0` across two rounds. |
| F02 | Timeline projection merges unmatched saved tools with journal-backed tools instead of dropping them. Sequence placement retains chronology. | `ResponseAudit` partial-journal regression and timeline suite. |
| F03 | One shared native alias registry drives categories and command verbs. `file_write` is an edit; AREX `search`/`visit` are web activity. Unknown connector tools retain a neutral category. Array queries are displayed as actual search terms. | `ResponseAudit`, mixed conversation and command renderer suites. |
| F04 | Single and batch transaction changes, native file creation, deletes and moves enter file review. Missing line diffs remain explicit. Repeated edits show recorded addition/deletion totals, rather than claiming a final net diff. | `WorkSummary`, batch/native write regression. |
| F05 | Inline citations, research context and source cards consume one hydrated evidence hook. Artifact requests are scoped and cached. Ambiguous IDs remain unlinked; missing metadata produces a visible notice even without recovered sources. | Real Chat artifact recovery regression and ambiguous-ID test. |
| F06 | Request completion is separate from process state. Owned cursor polling updates one process row, decodes stdout/stderr frames and exposes an owned Stop process control. Poll results coalesce with the launch. Missing live ownership is reported rather than presented as fresh completion. | `ProcessTranscript`; Rust ownership/stop regression and process suites. |

## Context and presentation

Compaction now creates a bounded semantic handoff from user requirements and observed tool results. It records completed call IDs, tool names/paths/outcomes, pending process handles and source URLs. Assistant notes are explicitly unverified. The handoff is saved with the verbatim archive and restored after history cache invalidation or reopening. Long current turns can compact whole settled tool rounds while retaining the original request, recent rounds and unfinished calls. This does not turn archived check results into fresh verification.

The response interface now uses compact chronological activity rows, subordinate reasoning and commentary, readable final prose, inset command output, source cards and an accurate file review. Successful commands opened automatically collapse on completion; deliberate inspection persists. Failures and approvals remain prominent. Closed reasoning and file diffs mount their expensive content on inspection. Historical turn arrays and callbacks retain identity so token updates can skip unchanged turns. Background approvals stay with their owning chat and retain the existing attention indicators.

Native visual review also corrected file-read rail alignment, generic web-query labels, internal connector wording and hidden successful exit-code text. Short windows use a smaller idle composer to leave more room for reading; existing composer controls remain accessible. The embedded capture tool supports light/dark themes, minimum-window geometry, rich transcript fixtures and an optional long history. It uses isolated data and an isolated X11 display, not a localhost browser preview.

## Validation

Verified results and final native captures are recorded below. Logs are retained under `.local/qa/audit-2026-11-08/`.

- Frontend full suite: **907 passed, 1 skipped**.
- Rust library full suite: **1,066 passed, 35 ignored**.
- Rust library check and TypeScript: **passed**.
- Embedded desktop build: **passed**, including the final frontend layout changes. The build reports a chunk-size warning.
- Native light/dark visual review: **passed** in the embedded app at 1280×840 and 760×560, including reasoning disclosure, command inspection, changed-file review and source cards. [Captures and provenance](ui-review/2026-11-08-response-fixes/README.md).
- Native long-history review: **passed** with 100 earlier turns plus the current turn; disclosure expansion was confirmed through accessibility state. Measurements include fixed waits and accessibility overhead, not frame timing.
- No paid inference, real-account authentication or browser preview was used. Ignored tests remain unverified.

Compatible inline thinking and literal XML tool fallback have separate policies: Chat Completions servers retain leading thinking support, while literal tool execution is restricted to the owned local template. Once the visible answer starts, literal `<think>` examples remain part of that answer. This has a character-by-character regression. Bounded instruction recovery also has a long-path/large-rule regression to ensure the notice itself cannot exceed the result budget.

The cancellation test now waits for both actual grouped polls to enter dispatch and measures interruption from cancellation, excluding model/setup time. The same bounded cancellation behavior and durable settlement assertions remain covered.

## Practical limits

These checks establish the repaired application paths and deterministic UI behavior. They do not establish absence of every possible application bug, real-provider acceptance for every model, sustained live-token frame timing, or comparative answer-quality parity with OpenCode/Claude Code. The native fixture is synthetic transcript replay. A controlled same-model task comparison remains necessary before claiming model-quality parity; no such claim is made here.
