# Conversation rendering

How LocalLM renders model output, tool activity, terminal output, code and
file changes. Everything here is frontend presentation: provider and harness
boundaries stay unchanged, and live events and persisted history share the
same components.

## Layers

| Layer | Files | Job |
|---|---|---|
| Semantic tokens | `src/tokens.css` | Activity, syntax, code-surface, diff and terminal colors for both themes. |
| Parsers | `src/lib/highlight.ts`, `src/lib/ansi.ts` | highlight.js wrapper with aliases + LRU cache; stateful ANSI-to-spans parser. |
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
- drops escape sequences truncated by a split stream chunk;
- preserves tabs, removes other control bytes.

ANSI colors resolve through `--terminal-*` tokens tuned per theme, so output
stays readable on both surfaces.

`CommandRunCard` renders stdout and stderr as separate labelled channels
(stderr is a channel, not a failure — status comes from the envelope:
`status`, `isError`, `exitCode`, `decision`). The viewport is bounded and
scrollable: while running it follows output only when the user is at the
bottom, and a "Jump to latest" pill appears after scrolling up. Shell
commands and `run_code` inputs are syntax highlighted; structured results
open in a highlighted JSON result tab; diffs use the shared `DiffView`.
`_artifactId`/`_originalBytes`/`_truncated` markers keep the artifact
inspect affordance.

## Diffs and file results

`DiffView` (`DiffLines` + `diffCounts`) is shared by `CommandRunCard`'s diff
tab and `WorkSummary`'s file-change review. Added/deleted/hunk lines use the
`--diff-*` tokens. `FileResult` shows path, total lines, SHA-256, a copy
action, highlighted content from `languageFromPath`, and raw request/result
under nested details.

## Streaming behaviour

- `MessageBody`/`MarkdownView` are memoized; `highlightCode` caches, so
  historical blocks do not re-highlight per token.
- `ActivityTimeline` keeps one reasoning disclosure per turn; expansion
  state survives streaming and live→persisted replays; durations display
  only from recorded event data.
- Terminal follow/auto-scroll honors scroll-up intent; disclosures are
  native `<details>` elements preserving their open state.

## Accessibility and safety

- AA contrast verified for category/syntax/diff foreground pairs (4.5:1);
  meaningful graphics pair color with icons and labels.
- Keyboard-focusable disclosures, named buttons (`Copy code block`, `Jump to
  latest output`, `Wrap long lines`), thin `--focus-border` indicator,
  reduced-motion respected via existing `prefers-reduced-motion` rules.
- Model/tool output is untrusted: Markdown renders via react-markdown (no
  raw HTML), code markup is escaped, source URLs pass `safeSourceUrl`,
  generated HTML/SVG is shown literally and never executed for display.
- Long lists are bounded: terminal viewport, code viewport and the
  completed-activity group cap DOM growth; full output lives in artifacts.

## Dependencies

highlight.js was already installed; the implementation uses
`highlight.js/lib/core` with the 26 grammars above
(no runtime theme CSS — colors come from `--syn-*` tokens). Shiki was
not adopted: it would add a dual-theme engine and WASM/grammar payload to
duplicate what the token system already provides. Math adds `remark-math`,
`rehype-katex`, `katex` and a direct `mdast-util-from-markdown` dependency.
Scoped package overrides keep both math plugins on the direct patched KaTeX
version instead of their older transitive version.

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
Completed activity now uses neutral, borderless disclosure rows with action
labels. Groups open during generation and remain inspectable afterward; pending
approvals, failures, cancellations and unknown outcomes stay outside collapsed
groups. Detailed code, terminal output, artifacts and raw results retain their
existing renderers and semantic colors.

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
