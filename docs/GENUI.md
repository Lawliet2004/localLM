# Generative UI

LocalLM renders official OpenUI Lang inside the existing Tauri chat. Auto is the
application default; Text keeps the normal answer path; Visual requests a visual
presentation. Settings → Models & runtime controls the application default. The
home and chat composers intentionally have no response-mode selector or override
editing. This control was removed by user request on 21 October 2026; do not
restore it from older plans or logs. Changing the Settings default never changes
how an existing message is interpreted.

## Generation boundary and streaming

The existing agent completes its ordinary answer, tools, research finalization,
and model-visible history first. `presentation.rs` then selects a compact library
with deterministic rules and, when appropriate, streams a separate **tool-free**
formatting request through the selected local/remote inference backend. Ordinary
Auto questions and Text turns incur no formatter call. This implementation uses
post-agent presentation, rather than injecting UI requirements into planning or
control generations. A visual turn therefore costs one additional inference pass.

The formatter receives only the component specification, current question, and
original answer. Tool schemas, internal UI state and history envelopes are excluded.
The authoritative answer stays in `messages.content`. The existing Tauri channel
carries presentation records independently of reasoning/tool/text events. Updates
are throttled around 80 ms and SQLite checkpoints around 500 ms. The frontend
accumulates the program and feeds the official Renderer with `isStreaming`.
Source is hidden behind an explicit disclosure. Original-answer access remains
available; interruption, incompatible versions, validation errors and render
exceptions fall back to that answer. No paid repair service is required.

Preflight reports the minimum presentation request separately from agent context.
The final formatter request is counted again against the configured context and
reserved output. It downgrades to Core, then Markdown if necessary, without
truncating history or increasing the user's context setting. Output reservations
below 512 tokens select text. The formatter uses Quick/no-thinking hints, disables
prompt reuse, and uses slot 1 when at least two inference slots are configured.
On a single-slot local runtime, formatting can displace the existing KV cache;
the agent's frozen system prompt and tool schema remain unchanged.

## Components and local interactions

Schemas live in `src/genui/catalog.ts` and `richCatalog.ts`; `library.tsx` attaches
React renderers. Components use existing LocalLM theme tokens, shared code and
diff primitives, and the existing Markdown styles. Generated Markdown has a
dedicated renderer that restricts links to supplied evidence and replaces images
with text references. Charting uses bounded responsive SVG rather than adding a
chart engine.

Surfaces, borders, action/status colors, typography and control radii come from
the existing design system. Grids and side-by-side diffs stack at container widths
of 480px or less; chat, canvas and visual answers establish their own containers.
Rows wrap and tables scroll independently. Focus and reduced-motion treatments
retain the existing token-based control language.

| Profile | Main capabilities |
| --- | --- |
| Core | Headings, text, cards, bounded layouts, sortable tables, lists |
| Data | Bar, line, scatter, pie/donut charts, diagrams, timelines, source cards |
| Education | Quiz, flashcards, slider/select/toggle/input, calculator, simulations |
| Coding | Highlighted code, search/collapse, line numbers, file trees, file previews, unified/side-by-side diffs |

Reusable deterministic simulations include sorting, binary search, LRU caching,
BFS/DFS, SQL joins (including duplicate-row semantics), and a one-dimensional
educational gradient-descent example. These are demonstrations, not execution of
generated code. Step, play/pause, speed and reset run locally. OpenUI's official
`useStateField`, `initialState` and `onStateUpdate` restore controls, quizzes and
simulations. Table sorting and chart selection also stay local.

Examples of accepted positional programs:

```text
root = Response([Heading("Supplied sales"), Chart("Sales", "bar", [{label:"Jan",value:10},{label:"Feb",value:15},{label:"Mar",value:25}])])
```

Consult the generated profile specifications for exact property order, optional
arguments and bounds. The runtime library identity is `locallm.openui/2`.
Not every component belongs to every prompt profile.

## Dependencies and generated specifications

Pinned runtime dependencies: `@openuidev/react-lang` 1.0.1,
`@openuidev/lang-core` 1.0.1 and Zod 4.6.5. Development-only
`@openuidev/cli` is 0.6.0. The full `react-ui` library was evaluated but is not
installed: its broader design system and overlapping dependencies are unnecessary.
No runtime Node server or additional chart/editor dependency is introduced.

Run `npm run generate:openui` after changing schemas. `build-genui-prompt.mjs`
invokes the installed official CLI for full/core/data/education/coding libraries:

```bash
npx --no-install openui generate ./src/genui/specification.ts \
  --export fullLibrary --prompt-options fullPromptOptions \
  --no-interactive --out ./src/genui/generated/full.txt
```

The `.txt` and `.spec.json` pairs derive from the actual schemas. Rust includes
static profile assets. Dev/build regenerate them; parity tests detect drift.
The full compatibility asset is copied to `src-tauri/src/genui_prompt.txt`.
Telemetry is disabled during generation; Renderer observability publishing is off.

## Persistence and compatibility

An additive startup migration creates:

- `message_presentations(message_id PRIMARY KEY REFERENCES messages ON DELETE CASCADE, record_json)`
- `conversation_presentation_modes(conversation_id PRIMARY KEY REFERENCES conversations ON DELETE CASCADE, mode_json)`

The global mode uses the separate `presentation.default` settings key. Existing
model, provider and tool preferences are preserved. Each record includes format,
format/library versions, original answer, raw program, interactive state, status
and diagnostic. State writes check conversation/message ownership, completion and
library identity. State is capped at 32 KiB, depth 12, 2,048 values and 256 array
items. A trigger invalidates presentation metadata when a finalized answer is
edited. Startup marks unfinished presentations interrupted. Deletion cascades.
Malformed metadata falls back to the original answer. Unsupported library versions
also fall back rather than reinterpreting old programs.

JSON export schema version 2 includes presentation records separately. Markdown
export and model history retain original factual text. Old Markdown conversations
need no rewrite; existing explicitly fenced OpenUI messages remain supported.
The independent SQLite envelope provides OpenUI initial-state/update semantics;
persistence envelopes are never appended to model-visible text.

## Security and grounding

Programs are untrusted. Validation uses the official tokenizer/parser plus bounded
AST/reference checks before materialization, followed by Zod property checks.
Limits include 64 KiB program text, 80 statements, 128 components, reference depth
16, bounded strings/children, 100 table rows and 200 chart points. Unknown
components, cycles, excessive expansion, arbitrary expressions, Query/Mutation,
functions and executable state definitions are rejected. There is no `eval`,
arbitrary JSX, automatic code execution or generated privileged host action.

Bounded local repair corrects named-to-positional arguments and singleton array
wrapping, plus at most four missing component closers before a parent array,
only when the complete result passes full validation. It invents no data or
missing references. Raw source remains available for inspection. Truncated or
unrepairable output returns to the original answer.

Buttons allow only bounded **visible conversation follow-ups** and credential-free
HTTPS URLs already present in the authoritative answer or actual source evidence.
Follow-ups use the ordinary submission path; any tools still pass through existing
permissions and audit logging. Unknown action types and filesystem/Tauri/data/JS
URLs are rejected. Generated file labels are display-only. Research links resolve
against actual evidence; chart labels and numeric data must occur in the original
answer. These checks do not prove full semantic equivalence of all generated prose.
Remote/local Markdown images become text references. CSP/capabilities were not
weakened. Tool parsing excludes OpenUI fences so literal tool syntax cannot execute.

## Verification and measurement

See [implementation verification](GENERATIVE_UI_IMPLEMENTATION.md) and the measured
JSON in `docs/benchmarks/`. Tests exercise the actual official Renderer, incremental
programs, fallbacks, state restoration, actions, grounding, simulations, mode races,
SQLite ownership/cascades/recovery and an isolated formatter SSE round trip.

The native acceptance harness starts the embedded Tauri/WebKit application on an
isolated display with a separate SQLite database. It exercises controls, restart,
light/dark/narrow layouts and optionally real CPU GGUF inference. It neither opens
a browser preview nor modifies the user's database. Browser Playwright acceptance
remains unexecuted under this workspace's explicit preview restriction.

Small-model reliability is conditional: the recorded Gemma E4B Q2 CPU sample
produced five raw programs that all needed bounded syntax repair; all five repaired
programs validated and rendered in tests. This is one sample per case, not a success
rate estimate or a claim about all 2B–4B models. Ordinary Markdown remains the
reliable fallback. Native acceptance is Linux-only; other desktop platforms and
remote-provider live visual inference have not been separately measured.

## Production hardening — 21 October 2026

The original answer remains authoritative. New message presentations now check
row/label/value associations within the same authoritative clause or Markdown row,
metrics and key/value pairs, literal code/diffs/paths, units when supplied as axis
labels, and citation-to-claim associations. This is conservative provenance checking,
not semantic proof of arbitrary prose. Ambiguous formatting falls back to the
original answer. Educational simulations and function plots are explicitly examples.

Streaming commits closed component calls and retains earlier valid content while a
new child is incomplete or ungrounded. It never completes an unfinished string or
invents data. Root-first references and completed named-argument syntax are supported.
The official renderer keeps its identity across these updates; actions stay disabled.
Duplicate explicit and implicit local state keys are rejected. Quiz DOM radio groups
are scoped to React instances, while stored quiz state remains scoped to each message.

New bounded experiences use the same library and tokens:

- `Diagram` lays out directed graphs with an accessible connection list; cyclic
  edges remain visible. Layout is display-only, bounded to 24 nodes/48 edges.
- `FileTree(paths, id?)` provides a saved collapsible hierarchy. `DiffViewer(diff,
  id?)` aligns replacement/addition/deletion groups within unified hunks and retains
  real line numbers and gaps. IDs distinguish multiple trees or diffs in one program.
- `Chart(title, kind, points, xLabel?, yLabel?)` supports supplied axis labels/units;
  scatter uses numeric scaled ticks and exposes both coordinates in its value table.
- `Slider("slope", "Slope", -2, 2, 1)` can drive
  `FunctionPlot("Linear", "slope", "linear", -2, 2)`. Only the built-in linear,
  quadratic and exponential families run. The parameter must reference a numeric
  Slider in the same program. There are no expression strings, generated functions,
  queries or executable bindings.
- `SeriesChart(title, [{name, points}, ...])` compares up to four supplied series
  with local visibility controls. It checks series/label/value provenance together.
- `EvidenceView(["source-id", ...])` renders host-collected titles/excerpts/dates and
  actual host claim-audit results. Model-authored verdicts are never metadata.
  Missing IDs remain unavailable; lexical matching is not independent verification.

These are additive library-v2 schemas: existing positional arguments keep their
meaning, new optional fields are appended, and new components need no SQLite
migration. Incompatible future library versions still fall back to the stored answer.

Settings changes update a still-mounted chat and its preflight; concurrent composer
selector removal is preserved. Existing per-conversation backend overrides still win
against a default change. Stale initial reads cannot revert a later preference save.

Formatter rounds now record `phase: "presentation"` in normal response metrics,
including actual provider token counters and duration. `presentation_end` reports
transport completion with rendering unverified. Parser/grounding/renderer rejection
uses an ownership- and program-checked command, stores a bounded host diagnostic and
adds `presentation_rejected`; it does not rewrite model history or prior audit rows.
Auto backs off after three failures in the latest five presentations for the same
model and endpoint; explicit Visual remains available to retry. Output/context limits
are retained. Two-slot runtimes use slot 1; one-slot formatting can displace the
agent KV cache, and this policy is recorded in the start event. No single-pass
strategy is enabled without evidence that it preserves answer/history guarantees.

Current proof and remaining gates: [completion tracker](audits/GENUI_COMPLETION_TRACKER.md).

### Host-bound chart facts

`SourceChart(id, tableIndex, kind)` references an exact numeric two-column table or contiguous colon-bullet
dataset in that message's authoritative answer. It supplies no model-authored labels, units or
numbers: the host derives those from literal cells and headers. The source-order
index is bounded to eight tables, 100 rows, 240-character labels and finite values
within ±1e12. Currency text, ambiguous widths, unsupported scatter coordinates and
oversized tables fall back. Pie/donut values must be nonnegative with a positive
total. Source data is scoped to the owning message, read-only, and carries no file,
network or tool authority. The extraction contract is version 1; changing index or
numeric interpretation requires a library version change, not silent replay.

Explicit chart requests now use a compact official `chart` profile. This is still a
separate formatter pass after agent completion. With supported source data, the model
chooses the SourceChart presentation; it does not transcribe measurements. This
profile contains no raw Chart constructor; unsupported source data stays text.
Scatter and multiple-series requests use the broader Data profile. The recorded
local tests distinguish schema repair from factual success: the first broad and
compact transcription attempts failed, while the host-bound table experiment passed
3/3 raw syntax, grounding and server-render checks. That small sample is not a
universal model reliability claim. Flowcharts remain in Core and ordinary coding
fixes mentioning charts remain Text in Auto.

Linked controls register defaults through the official OpenUI default-value hook.
Function plots wait for the linked value while streaming and honor a non-1 initial
slider value in either component order. Persisted values take precedence; the
formatter cannot execute expressions to produce a binding.

`SourceCode(fenceIndex, path?)` similarly presents the exact body of a closed code
fence from the authoritative answer through the shared highlighted FilePreview.
It preserves literal bytes, including the final newline, and never executes them.
The index is bounded to 16 eligible blocks, each at most 16,384 characters; an
optional display path must match the explicit filename heading associated with
that exact block. A path mentioned elsewhere is insufficient. Incomplete fences and
unknown indices fall back. Exact-code requests use a compact official `code` profile with SourceCode, Heading,
Text and Response. General coding requests retain the broader Coding profile.
Coding formatters prefer this binding over duplicating code into another string.

Global presentation preference writes are serialized across Settings instances.
A successful save still notifies mounted chats if its Settings modal closes while
the write is pending; conversation overrides remain authoritative.
