# Conversation workspace upgrade — 22 October 2026

This is an implementation in the existing React/Tauri application. It preserves
warm paper and green charcoal themes, IBM Plex, semantic rendering tokens,
Settings-only response presentation, providers, permissions, project handling,
OpenUI, artifact actions, and native transport/storage contracts. No package or
lockfile changes, borrowed source, or parallel rendering engine. The follow-up
adds an optional native journal-cursor notification without changing existing
commands, tool envelopes or execution/approval contracts.

## Audit and implementation

The source audit covered AGENTS.md, DESIGN.md, PRODUCT.md, RENDERING.md,
IMPLEMENTATION_REPORT.md, package.json, the requested conversation/rendering
components, App's streaming callback, related hooks, styles, parsers and tests.
The initial Git status was clean; no reset, stash, clean or commit was performed.

Already working: readable 744px standard conversation measure (824px preference
minus 80px), open assistant prose, compact user bubbles, composer controls,
message copy/retry, scoped native cancellation, math/citations and untrusted-output
restrictions, syntax grammar aliases/cache/size caps, diff tokens, artifact
inspection, saved appearance preferences and turn-scoped journal replay.

Changes implemented:

- **Chronological activity:** only adjacent successful reads/searches fold.
  Commands, edits, web/MCP operations, approvals and unsuccessful outcomes stay
  individually inspectable in their original position. Multi-read labels count
  distinct recorded paths; other groups count actual operations. Duplicate call
  deliveries are removed and calls are sorted by journal sequence. A recorded
  call no longer invents a pending approval when no decision was recorded.
- **One compact row treatment:** command/tool disclosures are borderless in the
  conversation, with category icons and textual statuses, restrained running
  feedback and visible edit counts. Detail surfaces retain code, output, exact
  commands, cwd, connector identity, parameters, artifacts, copy and supported
  stop actions. Failures and approvals open automatically. Search counts no
  longer guess from the number of log lines.
- **Streaming transport:** a small transient `ToolOutputBuffers` store uses
  `useSyncExternalStore` keyed by conversation and tool message. Chunks append
  stdout/stderr without mapping messages or parsing/stringifying their JSON.
  Only that tool is notified, at most once per 32ms burst. Started/finished events
  still update metadata immediately; finish synchronously captures all buffered
  bytes. The existing persisted reload remains authoritative; error recovery
  retains partial bytes, and turn cleanup cancels pending timers and drops buffers.
- **Terminal output:** one shared incremental ANSI parser handles both live and
  replay data, with separate channel state, split escape/CRLF recovery, progress
  rewrites and control filtering. Completed line objects are reused. Folded live
  output shows a 30-line tail; scrolling up freezes the window, and Latest resumes
  even after completion. Expanded logs page at 500 lines per channel rather than
  mounting an unlimited log. Every line remains browsable; Copy output keeps the
  full source, and captured artifacts retain their inspection action. Copy errors
  are explicit. Output tabs no longer reset merely because stdout changes.
- **Markdown/code:** module-stable renderer identities preserve DOM, wrap/copy
  state and selection during appended prose and the live-to-complete transition.
  Memoized fences keep unchanged code out of highlighting work. Code line arrays
  are reused. Artifact Markdown now uses the same renderer as conversation prose,
  including its existing safety and citation defaults, rather than a second
  react-markdown configuration. OpenUI and grounded visual-prose restrictions remain.
- **Diffs/work summary:** shared diffs show actual old/new hunk coordinates,
  syntax-highlight changed lines through the existing safe highlighter, retain
  +/- prefixes, and count source beginning with ++/-- correctly. Summaries retain
  full paths, real recorded counts and inspection, with smaller icons and spacing.
- **Scrolling/accessibility:** ResizeObserver follows tool-local output, images
  and layout resizing only while following is enabled. Opening a disclosure
  pauses following; scrolling to the bottom or Latest resumes it. Conversation
  switching resets follow state. Log pages/regions and diff inspection are named
  and keyboard reachable; existing focus tokens and reduced-motion policies apply.
  Narrow rows wrap long command text and keep attention/stop controls available.

## External source review and architecture decisions

All references were inspected at the following exact revisions. Only interaction
patterns informed the implementation; no external source was copied, so no new
source attribution notice or dependency pin is required.

| Reference | Revision and sources | License | Decision |
| --- | --- | --- | --- |
| [OpenCode](https://github.com/anomalyco/opencode) dev | `c93aaae37157d785f76a8e9a9243a6560e286468`; session-ui `message-part.tsx`, `basic-tool.tsx`, `tool-status-title.tsx`, `tool-count-summary.tsx` | MIT | Compact disclosure titles, selective grouping and actual-state labels. `message-part.tsx` is now a re-export; requested `part-default-open.ts` is absent at this revision. No SolidJS/motion transplant. |
| [assistant-ui](https://github.com/assistant-ui/assistant-ui) | `94d25fa1362cb685c688dbc1553f47dfd20b07bc`; thread documentation and Message, Composer, ActionBar, MessagePart, BranchPicker, ThreadList primitive reference MDX | MIT | Adopt follow-only-at-bottom and pause-on-disclosure behavior. Retain existing components: a custom runtime adapter would duplicate state ownership without solving the identified output hot path. |
| [Streamdown](https://github.com/vercel/streamdown) | `c5e44d84cfcc10427ae380863a562e8127ae3b86`; code-block/table sources, `lib/parse-blocks.tsx`, incremental parser tests | Apache-2.0 | Retain react-markdown, citation/math/OpenUI pipeline. Stabilize component identities and memoized fences. Streamdown's block lexer/remend/plugin system would add another compatibility boundary; unsafe naive blank-line splitting was avoided. |
| [TanStack Virtual](https://github.com/TanStack/virtual) | `887e97172207c28d12bf9a0df8a842e261565a42`; React adapter and measurement/anchoring behavior | MIT | No virtualization dependency: no native layout profile establishes a chat-history threshold, and recycling risks selection/focus and dynamic expansion. Bounded terminal pages address the demonstrated unbounded DOM path. |
| [shadcn/ui](https://github.com/shadcn-ui/ui) | `ba1503c64472b82256d9de95e2e62c5901a38e8b`; aria registry collapsible/dialog components | MIT | Keep existing native details, buttons and dialogs. No framework or parallel design system. |

`useSessionActivity` remains a cursor-based persisted-journal loader. Native
`emit_session_event` now emits an optional `locallm://session-activity`
notification only after a successful commit, containing the conversation ID and
committed sequence number. The UI subscribes before its initial recovery load,
filters consumed cursors, and immediately reads rows through the existing command.
It does not carry another copy of model/tool output. Existing 750ms active polling,
pagination, deduplication, loading coalescence and teardown remain recovery paths
for missed events and older running backends. This adds no UI authority to detached
tools and changes no execution, approval, provider context or envelope contract.

## Measurements

Linux, Node v22.22.1. Synthetic ANSI log: 500 chunks, 276,000 bytes, 6,000 output
lines. Median of three samples per implementation, same fixture/process. Baseline
was `git show HEAD:src/lib/ansi.ts` saved outside the repository before comparison.

| Measurement | Result |
| --- | ---: |
| Previous full-buffer parse on every chunk | 2,485.59 ms |
| New accumulated-prop adapter used by CommandRunCard | 91.98 ms |
| New direct incremental parser | 26.90 ms |
| Characters fed into incremental parser | 276,000 |
| 100 synchronous chunks in subscriber regression | One affected-tool render; zero parent or sibling renders |
| 16 completed assistant responses during five later updates | Zero additional Markdown parses (existing regression retained) |
| Expanded 1,600-line log | At most 500 rendered lines per channel; all pages and full copy preserved |

These measure parser CPU and deterministic React render/DOM counts, not native
frame rate, first visible token/tool latency, model inference or tool execution.
The accumulated adapter still compares prefixes and snapshots the line index;
this is not a claim of constant-time total rendering. Reproduce using:

```sh
node --import ./scripts/register-test-loader.mjs scripts/benchmark-terminal.mjs
# Optional previous parser module (absolute .ts path):
LOCALLM_ANSI_BASELINE=/absolute/path/ansi.ts node --import ./scripts/register-test-loader.mjs scripts/benchmark-terminal.mjs
```

## Verification and limits

| Check | Actual outcome |
| --- | --- |
| `npm run typecheck` | Passed |
| Frontend final: `npx vitest run --maxWorkers=1 --testTimeout=15000` | 845 passed, 1 existing opt-in skipped; 117 files passed, 1 skipped |
| `npm run build` | Passed; existing >500 KB chunk advisory remains |
| `cd src-tauri && cargo check --lib` | Passed |
| `cargo test --lib -- --test-threads=1` | 977 passed, 4 observation tests failed, 19 ignored; details below |
| Browser/native visual and interaction checks | Not run under repository preview restrictions |

Earlier intermediate runs exposed assertions for the old all-tools group and two
new test-harness mistakes; those were corrected. No final frontend failures remain; Rust failures are recorded below.
The App integration regression exercises started/chunks/finished, both output
channels, persisted-history replacement and buffer disposal. It uses a mocked
transport and is not a native execution claim.

The mechanical Impeccable detector returned `[]` for all changed UI targets.
A separate finish reviewer returned **ship at code-only scope**, after finding
and verifying a fix for Latest on a paused tool that completed before resuming.
This is explicitly not visual approval.

No localhost/browser preview was attempted, in accordance with AGENTS.md.
Native screenshots, actual wide/laptop/narrow theme comparisons, 200% zoom,
increased text, computed contrast, keyboard-only interaction and native streaming
acceptance were not performed. jsdom tests cover semantics/state/scroll intentions,
not browser layout. The existing canvas getContext notice and large bundle warning
are disclosed rather than treated as native evidence.

## Follow-up: complete integration inside localLM

The second implementation pass closes the remaining source-level integration gaps:

- `MarkdownBlockBuffer` uses the already installed CommonMark parser to retain
  safe completed boundaries and recheck a two-node mutable tail. Each block is
  rendered by the same canonical ReactMarkdown/citation/math/code/OpenUI pipeline.
  Definitions (including multiline and escaped labels), footnotes, math and HTML
  conservatively retain whole-document semantics. No new Markdown engine ships.
  Tests compare every streamed prefix of twelve fixtures with canonical GFM output.
- Completed prose within a growing answer now avoids reparsing as well as code.
  Stable offset keys, generic first/last margins and caret placement preserve the
  compact user bubbles and normal reading rhythm.
- Completed command and file details mount on first inspection; raw file JSON
  mounts only when its own disclosure opens. Inspected contents stay mounted on
  collapse, preserving controls and scroll state. ANSI work is deferred while
  command output is collapsed or a different tab is selected.
- Diffs fold long unchanged regions with three context lines on either side.
  Changed lines and real coordinates remain visible; the keyboard-focused toggle
  stays mounted through expansion/collapse.
- Parsed tool records and normalized journal rows use WeakMap identity caches.
  Persisted-call matching is scoped by run ID or call timestamp interval, so reused
  provider IDs cannot attach a later recorded result to an earlier run.
- Native cursor notifications now update the work log immediately after commit,
  while retaining the existing journal as its only replay authority.

Synthetic CommonMark boundary benchmark (Node v22.22.1, median of three runs):
100 appended updates after 26,290 completed characters took **25.25ms** with the
incremental buffer versus **797.07ms** parsing the entire document each time.
Characters fed to that parser fell from 2,750,200 to 156,202. This measures boundary
parsing only, excluding React, GFM/math rendering, layout, Tauri and inference.
Reproduce with `node --import ./scripts/register-test-loader.mjs scripts/benchmark-markdown.mjs`.

The second mechanical detector returned `[]`. A separate review found and verified
fixes for multiline/escaped reference labels, generic block margins, and diff
focus retention. Final disposition: accepted at **code-only scope**. Native/visual
approval is not claimed.

Follow-up verification:

- TypeScript typecheck, production web build, diff whitespace and mechanical
  detector passed. Final frontend suite: **845 passed / 1 existing opt-in skip**,
  with one worker and a 15s test timeout during native compilation. Earlier default
  runs under compilation load hit a 5s settings test timeout and a lazy OpenUI load
  timeout; the final constrained run passed both without product-code workarounds
  or repository timeout changes. The earlier artifact assertion was updated to
  explicitly open the newly deferred tool disclosure.
- Rust check passed, including the additive event API. The notification payload
  test passed in both full runs. The first parallel library run reported
  **976 passed / 5 failed / 19 ignored**. Serial rerun reported
  **977 passed / 4 failed / 19 ignored**. The cancellation test passed serially.
  Four observation tests remain failing on this host:
  `development_tools::tests::observations_finish_without_report_polling_and_remain_historical`
  (timeout), `development_tools::tests::source_bound_checks_become_stale_and_enforce_ownership`
  (missing completion evidence),
  `process_tools::tests::a_stop_is_acknowledged_before_the_observation_is_sealed`
  and `verification_observer::tests::native_events_detect_modified_then_restored_sources`
  (both `Too many open files` while initializing a source observer). These failures
  were not skipped, reclassified as passing or addressed by changing unrelated
  observer logic or host kernel limits.
- `npm run tauri build -- --no-bundle` failed before building because its filesystem
  watcher could not initialize (`Too many open files`). A direct Cargo release
  build with `tauri/custom-protocol` uses embedded production assets and avoids
  that CLI watcher. The native release compilation succeeded; the final artifact
  is `src-tauri/target/release/locallm`. The final release build succeeded; binary
  inspection confirmed the current JS/CSS asset keys and native event name.
  Existing automation license resources accompany the executable under its
  `resources/automation/licenses/` directory. This is an unbundled executable, not an installer
  or a native interaction/screenshot acceptance claim.


Remaining limits: global Markdown constructs use the whole-document fallback;
chat history is not virtualized; expanded code and changed diff regions may be
large; full output buffers stay in memory until turn cleanup. A previously running
native process needs a restart to load the Rust notification code; the UI still
recovers through polling on that older process. No native first-visible-response,
frame-rate, inference-speed or backend execution improvement is claimed.

## Exact changed-file inventory

- `DESIGN.md`
- `docs/RENDERING.md`
- `docs/UI_UPGRADE_2026-10-22.md`
- `scripts/benchmark-markdown.mjs`
- `scripts/benchmark-terminal.mjs`
- `src-tauri/src/chat.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/sessions.rs`
- `src/App.test.tsx`
- `src/App.tsx`
- `src/components/ActivityTimeline.test.tsx`
- `src/components/ActivityTimeline.tsx`
- `src/components/ArtifactViewer.tsx`
- `src/components/Chat.performance.test.tsx`
- `src/components/Chat.test.tsx`
- `src/components/Chat.tsx`
- `src/components/CodeBlock.tsx`
- `src/components/CommandRunCard.test.tsx`
- `src/components/CommandRunCard.tsx`
- `src/components/DiffView.test.tsx`
- `src/components/DiffView.tsx`
- `src/components/FileResult.test.tsx`
- `src/components/FileResult.tsx`
- `src/components/MarkdownView.test.tsx`
- `src/components/MarkdownView.tsx`
- `src/components/MixedConversation.test.tsx`
- `src/components/SessionActivity.test.tsx`
- `src/components/TraceIcon.tsx`
- `src/components/WorkSummary.tsx`
- `src/lib/ansi.test.ts`
- `src/lib/ansi.ts`
- `src/lib/api.ts`
- `src/lib/markdownBlocks.test.tsx`
- `src/lib/markdownBlocks.ts`
- `src/lib/toolOutput.test.tsx`
- `src/lib/toolOutput.ts`
- `src/responses.css`
