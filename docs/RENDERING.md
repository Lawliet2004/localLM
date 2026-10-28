# Conversation rendering

How LocalLM renders model output, tool activity, terminal output, code and
file changes. Rendering stays in the frontend, with optional Rust journal
notifications supplying lightweight invalidation. Provider and harness boundaries
stay unchanged, and live events and persisted history share the same components.

## Layers

| Layer | Files | Job |
|---|---|---|
| Semantic tokens | `src/tokens.css` | Activity, syntax, code-surface, diff and terminal colors for both themes. |
| Parsers | `src/lib/highlight.ts`, `src/lib/ansi.ts`, `src/lib/markdownBlocks.ts` | highlight.js wrapper with aliases + LRU cache; stateful ANSI-to-spans parser; conservative Markdown block boundaries. |
| Shared primitives | `src/components/CodeBlock.tsx`, `DiffView.tsx`, `MarkdownView.tsx` | One code frame, one diff renderer, one Markdown renderer. |
| Surfaces | `Chat.tsx`, `ActivityTimeline.tsx`, `CommandRunCard.tsx`, `FileResult.tsx`, `ArtifactViewer.tsx`, `WorkSummary.tsx` | Map structured records to the primitives. |

## Two color axes

Activity **category** and execution **status** are separate axes. A failed
command keeps its blue command icon while its badge turns red.

Category (`--cat-*` foreground, `--cat-*-soft` tint, `--cat-*-border`):

| Category | Token | Hue |
|---|---|---|
| Exposed reasoning | `--cat-reasoning` | violet |
| Commands / code execution | `--cat-command` | blue |
| Web search and browsing | `--cat-web` | cyan |
| File reads / exploration | `--cat-read` | muted teal |
| File edits | `--cat-edit` | amber |
| Generic / MCP tools | `--cat-tool` | slate |

Status reuses the existing `--accent` (success/finished), `--warn`
(awaiting approval) and `--error` (failed/denied) tokens; interrupted runs
use a neutral badge labelled "Stopped · outcome unknown". Categories pick up
icons via `TraceIcon`/`toolKindOf`, card left borders via
`data-kind` on `.command-run-card`, and labels via `displayVerb`; status only
changes badges, never the category color.

All `--cat-*` foregrounds measure ≥ 4.5:1 against their `--*-soft`
backgrounds and the message surface in both themes (light: violet 6.22, blue
6.21, cyan 5.21, teal 4.82, amber 5.64, slate 5.25; dark values are the
pastel variants in `tokens.css`). Color is always paired with an icon and a
text label.

## Code rendering

`src/lib/highlight.ts` registers a fixed highlight.js grammar subset
(typescript, javascript, rust, python, bash, json, xml, css, yaml, sql,
markdown, diff, ini, plaintext, c, cpp, csharp, java, go, kotlin, swift,
ruby, php, dockerfile, makefile, powershell) and resolves common aliases
(`ts`, `tsx`, `py`, `sh`, `yml`, `md`, `patch`, `html` → xml, `toml` → ini,
`c++` → cpp, `cs` → csharp). Auto-detection is never used, which keeps streaming highlighting
bounded. `languageFromPath` infers a grammar for file reads and artifacts.

`highlightCode` returns escaped markup: grammar output is escaped by
highlight.js, the fallback path is escaped in `escapeHtml`, and both feed
`dangerouslySetInnerHTML` only after escaping. Results are memoized in a
160-entry LRU so completed blocks are not re-highlighted on each streamed
token. Blocks over 100 000 chars skip highlighting (`truncated` flag shows a
"plain" note); blocks over 20 000 chars that are still `streaming` defer
highlighting to the completed render.

`CodeBlock` is the single frame: language label, wrap toggle (only when a
line exceeds ~120 chars), copy button (always copies the exact source, never
the markup) and optional actions such as Canvas. Syntax colors come from
`--syn-*` tokens; the frame from `--code-bg/-fg/-border/-header-bg`.

## Markdown

`MarkdownView` is the one renderer for assistant prose, user messages and
exposed reasoning. It keeps `react-markdown` + `remark-gfm` + the citation
plugin; code fences map to `CodeBlock` and `openui` fences keep the lazy
`GenUiBlock`. Unclosed fences from a still-streaming block already parse as
code to end of input, so incomplete Markdown needs no rewriting. Tables are
wrapped in a horizontal scroller so narrow windows do not overflow.

`MarkdownBlockBuffer` uses the existing `mdast-util-from-markdown` dependency
only to find conservative blank-line block boundaries. Rendering still uses
memoized blocks through the canonical ReactMarkdown pipeline. A safe prefix can
be reused while the two trailing AST nodes remain mutable for continuations,
lists and setext headings. Source-offset keys preserve component identity.
Global reference definitions, including multiline or escaped labels, footnotes,
math and HTML conservatively fall back to whole-document rendering so later
content can still affect earlier content. Source replacement resets the buffer.
Generic first/last block margins and streaming-caret placement preserve the
existing prose and user-message bubble rhythm across block wrappers.

Visual-answer prose uses this same renderer, with its existing evidence-only
link policy and image restrictions. Malformed citation metadata falls back to
plain answer text. Table scrollers and code viewports are keyboard focusable.

Math uses `remark-math` and `rehype-katex`, with bundled KaTeX styles/fonts,
untrusted commands disabled, and expansion/size limits. Inline math uses
`$$…$$` or `\(…\)`; display math uses standalone `$$` lines or `\[…\]`.
Single dollars remain literal so prices such as `$5 and $10` are preserved.
`mathMarkdown.ts` normalizes LaTeX delimiters only outside Markdown code,
HTML, links and images; unclosed fences and copied source remain unchanged.

`responses.css` carries answer typography, responsive tables, larger image
previews and streaming-marker placement. Default answer text is 16px; saved
appearance preferences still govern font, size, width and density. Code frames
use `--code-*` in desktop overrides as well as shared primitives.

## Terminal output

`src/lib/ansi.ts` parses output into per-line styled spans. It:

- applies SGR colors and bold/dim/italic/underline, with all resets;
- carries SGR state across line boundaries (a color set mid-line persists
  until reset);
- collapses `\r` progress rewrites to the final segment and applies `\x08`;
- strips non-SGR sequences — CSI cursor/erase/private modes (incl. `?25l`),
  OSC hyperlinks/titles, charset and two-byte escapes;
- buffers incomplete escape sequences across live chunks and omits unfinished
  sequences from displayed output;
- preserves tabs, removes other control bytes.

ANSI colors resolve through `--terminal-*` tokens tuned per theme, so output
stays readable on both surfaces.

`AnsiStreamParser` is the shared incremental parser for live and accumulated
output. Each channel owns its parser state. `AccumulatedAnsiParser` feeds only
new bytes when source grows and resets when source is replaced or replayed;
completed line objects retain their identities. `parseAnsi` uses the same core
for one-shot history parsing.

`CommandRunCard` renders stdout and stderr as separate labelled channels
(stderr is a channel, not a failure — status comes from the envelope:
`status`, `isError`, `exitCode`, `decision`). The viewport is bounded and
scrollable: while running it follows output only when the user is at the
bottom, and a "Jump to latest" pill appears after scrolling up. Shell
commands and `run_code` inputs are syntax highlighted; structured results
open in a highlighted JSON result tab; diffs use the shared `DiffView`.
`_artifactId`/`_originalBytes`/`_truncated` markers keep the artifact
inspect affordance.

Folded output shows at most 30 lines; streamed output follows the latest tail
while following is active. Expanded channels expose up to 500 lines per page
with Previous/Next controls and source line numbers. Browsing pauses following;
Copy retains the full source rather than just the visible page. stdout and
stderr remain independent channels.

Completed command details mount on first inspection and retain local state
afterward. ANSI parsing runs only when the terminal tab is expanded, so collapsed
or nonterminal channels do not process output merely because a row rerenders.

## Diffs and file results

`DiffView` (`DiffLines` + `diffCounts`) is shared by `CommandRunCard`'s diff
tab and `WorkSummary`'s file-change review. Added/deleted/hunk lines use the
`--diff-*` tokens. Hunk rows carry old/new source coordinates; additions and
deletions keep their literal markers and use path-inferred syntax highlighting
within the existing size limit. `FileResult` shows path, total lines, SHA-256, a copy
action, highlighted content from `languageFromPath`, and raw request/result
under nested details.

Unchanged-context runs longer than eight lines keep three lines at each edge
and fold the middle behind a labelled Show/Collapse toggle. Expanding retains
old/new source coordinates, and the toggle keeps its DOM identity so keyboard
focus survives the change. File contents and raw request/result details defer
mounting until first inspection, then retain state across closing and reopening.

## Streaming behaviour

- `MessageBody`/`MarkdownView` are memoized; `highlightCode` caches, so
  historical blocks do not re-highlight per token.
- Markdown element component identities are stable across updates and the
  live-to-complete transition, preserving code wrap and disclosure state.
  `ArtifactViewer` uses `MarkdownView` for Markdown previews as well.
- `src/lib/toolOutput.ts` holds transient stdout/stderr buffers with subscriptions
  scoped to one conversation/tool. Output notifications batch at 32ms; chunk
  delivery avoids rewriting the message array and parsing/serializing tool JSON.
  Lifecycle metadata still updates immediately. Completed messages and persisted
  journals remain authoritative; transient buffers are cleared at turn cleanup.
- `ActivityTimeline` interleaves one reasoning disclosure per recorded model
  round with commentary and tool calls. Exact-prefix matching preserves older
  or incomplete journals without duplicating reasoning; a live trailing round
  keeps its disclosure identity when its journal arrives. Durations display
  only from matching recorded event data. Completed-round reasoning is journaled
  before backend planning/finalization exits and tool validation.
- WeakMap caches reuse parsed immutable tool records and normalized replay rows.
  Matching uses run identity or call timestamp intervals to keep reused provider
  call IDs from crossing runs; duplicate journal delivery keeps row order.
- Terminal follow/auto-scroll honors scroll-up intent; disclosures are
  native `<details>` elements preserving their open state.
- Chat observes content resizing while following. Scrolling up or opening a
  disclosure pauses following for reading; Latest resumes it and stays available
  after generation completes. A conversation change resets following.

## Journal delivery

Rust may emit `locallm://session-activity` after a journal append succeeds.
Its payload contains only `conversationId` and the committed `seq` cursor,
without duplicating model/tool payloads. `api.onSessionActivity` filters by
conversation and triggers the existing cursor loader; the journal remains the
authority. Subscription precedes recovery loading, and cursor reads merge and
deduplicate events in sequence. Polling remains for missed notifications and
older binaries. Teardown handles a listener registration resolving after the
conversation has changed; the teardown race has a regression test.

## Accessibility and safety

- AA contrast verified for category/syntax/diff foreground pairs (4.5:1);
  meaningful graphics pair color with icons and labels.
- Keyboard-focusable disclosures, named buttons (`Copy code block`, `Jump to
  latest output`, `Wrap long lines`), thin `--focus-border` indicator,
  reduced-motion respected via existing `prefers-reduced-motion` rules.
- Model/tool output is untrusted: Markdown renders via react-markdown (no
  raw HTML), code markup is escaped, source URLs pass `safeSourceUrl`,
  generated HTML/SVG is shown literally and never executed for display.
- Terminal DOM is bounded by the 30-line fold or 500-line channel page; code
  viewports remain scrollable. Exploration groups preserve their individual
  rows without a second nested scroll cap. Large backend results retain the
  existing artifact capture and inspection contract.

## Dependencies

highlight.js was already installed; the implementation uses
`highlight.js/lib/core` with the 26 grammars above
(no runtime theme CSS — colors come from `--syn-*` tokens). Shiki was
not adopted: it would add a dual-theme engine and WASM/grammar payload to
duplicate what the token system already provides. Math adds `remark-math`,
`rehype-katex`, `katex` and a direct `mdast-util-from-markdown` dependency.
Scoped package overrides keep both math plugins on the direct patched KaTeX
version instead of their older transitive version.
The 22 October incremental-rendering follow-up reuses these dependencies and
adds none.

## Tests

- `src/components/MarkdownView.test.tsx` — mixed rich answers, equations,
  incomplete streaming content, corrupt citations, literal HTML and visual
  grounding restrictions.
- `src/lib/mathMarkdown.test.ts` — model delimiters and protected source ranges.

- `src/lib/ansi.test.ts` — SGR/attributes, resets, cross-line state, `\r`,
  `\x08`, split-truncated and unsupported sequences.
- `src/lib/highlight.test.ts` — aliases, path inference, fallback escaping,
  size cap, memoization.
- `src/components/CodeBlock.test.tsx` — frame, highlighted spans, exact
  copy, wrap toggle, streaming deferral.
- `src/components/CommandRunCard.test.tsx` — status badges, exit codes,
  stderr channel, ANSI stripping/following, diff/code/result tabs, artifacts.
- `src/components/MixedConversation.test.tsx` — deterministic mixed
  conversation fixture (prose, reasoning, ok/failed commands, code, search,
  file read, diff, artifact, unknown tool) plus malformed/legacy payloads,
  interrupted runs and unfinished fences.

## Inline work log — 21 October 2026

The Codex screenshot references supersede the earlier answer-first card layout.
Completed activity uses neutral, borderless disclosure rows with action labels.
The 22 October refinement groups only adjacent successful reads, searches and
exploration, collapsed by default and inspectable afterward. Commands and edits
always retain separate chronological rows, as do running work, pending approvals,
failures, cancellations and unknown outcomes. Detailed code, terminal output,
artifacts and raw results retain their shared renderers and semantic colors.

When non-final `model_response` events exactly match the saved answer prefix,
`ActivityTimeline` interleaves commentary and tool groups in journal sequence.
Unmatched or older journals preserve the whole answer without replaying extra
text. Visual answers retain their complete presentation record. Commentary
fragments do not duplicate the final answer's presentation or metrics.

`WorkSummary` shows recorded duration, then one edited-files panel after the
answer, grouping repeated edits by path. Counts sum recorded diffs (rather than
claiming a Git net change). View changes exposes each recorded diff; file rows
can also expand individually, and the list can collapse. No undo action is shown
without a corresponding supported operation. The compact summary uses shared
diff foreground tokens in both themes.

Verification: focused conversation tests, the frontend suite, TypeScript and a
production build. Computer-use inventory exposed no native LocalLM window;
localhost browser preview remains excluded by repository instructions. Exact
pixel matching and native runtime appearance are therefore unverified.

## Refinement evidence — 22 October 2026

The existing visual system and 744px reading width are retained; this extension
adds no packages or palette/theme changes. The work summary uses a smaller icon,
tighter file rows and compact totals, keeping full paths and recorded-diff counts.
The mechanical detector returned `[]`. Review disposition: **ship at CODE-ONLY
scope** after fixing the Latest-after-completion regression, without visual
approval. No new screenshots, native/browser inspection or zoom/accessibility
visual acceptance are available; localhost preview remains prohibited. Synthetic
parser checks do not establish overall UI performance or model latency. Final
test outcomes and implementation measurements belong to the implementation
report rather than an inferred visual acceptance here.

The follow-up adds incremental Markdown boundaries, deferred inspected details,
unchanged diff context folding, replay caches and optional committed-journal
notifications. The reviewer accepted the final fixes at CODE-ONLY scope and the
detector again returned `[]`. Screenshots and native interaction approval remain
unavailable. Exact measurements, test failures and build outcomes are recorded
separately in the implementation report; earlier pinned source references remain
evidence for their initial-pass scope.

## Round chronology — 25 October 2026

Reasoning, intermediate commentary and tool groups follow journal sequence,
including duplicate or out-of-order delivery. Mixed exploration summarizes
searches and distinct files; commands and attention states remain separate.
Partial journals retain unmatched tool messages instead of hiding them. Working
commentary is borderless and exposed reasoning uses upright text. The existing
status shimmer remains the live-activity indicator. No localhost preview was
used; screenshot fidelity and native interaction remain unverified.

The current surface specification is `ACTIVITY_DESIGN.md`. Run/step/call identity
now owns tool positioning; unjournaled tools use recorded timestamps. The shared
`src/lib/toolResult.ts` adapter reads native mutation arrays and restores owned
artifact results for file summaries. `verification_status` events distinguish
running checks from completed test evidence. Verification and remaining native
acceptance limits are recorded in `AGENT_LOOP_RELIABILITY_2026-10-25.md`.
