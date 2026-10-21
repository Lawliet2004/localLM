# LocalLM generative UI: independent working-tree audit

> Superseded product requirement (21 October 2026): the user removed the
> `Default (auto)` response-mode selector and composer override editing.
> References below to implementing, styling or validating that selector are
> historical and must not be followed. Presentation preferences are Settings-only.
> See `AGENTS.md` and `DESIGN.md`.


Audit date: 21 October 2026. Repository: `/home/papan/Desktop/Projects/localLM`.
Git HEAD at inspection: `5eb2ba3d135725dc0a9bc6fb7156bf983e25d718`.
There were 434 dirty Git status entries before adding these audit documents. Do
not confuse HEAD with the implemented state: much of the application and GenUI is
uncommitted. No application implementation was changed during this audit.

## Verdict

**Not ready for full production acceptance.** Substantial GenUI implementation
exists. There are reproducible correctness defects, incomplete richer experiences,
and missing verification. More urgently, the current Rust working tree does not
compile. Previous successful Rust/native runs remain historical evidence, not
proof of the current source state.

Fresh checks:

| Check | Current audit result |
| --- | --- |
| `npm run typecheck` | Passed |
| `npm test -- --run` | 749 passed, 1 skipped; 108 passing files |
| `npm run build` | Passed; large-chunk warning remains |
| `cargo test --manifest-path src-tauri/Cargo.toml` | Compilation blocked by nine missing modules; tests did not run |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --message-format=json -- -D warnings` | Compilation failed; nine missing-module errors plus two Clippy diagnostics per target. Not a complete lint assessment |
| Static Impeccable detector on GenUI/default-mode/hook sources | `[]`; this did not detect the semantic defects below |
| Read-only parser/component probes | Confirmed grounding, arithmetic, streaming, radio grouping and scatter issues |
| Current native application build/acceptance | Blocked by the Rust compilation failure; old executable must not be treated as current source |
| Browser/Playwright | Not run; explicitly prohibited by workspace AGENTS.md |

Raw logs and the executable read-only probe are in
`.local/qa/genui-audit-2026-10-21/`. Portable reproduction inputs and outputs are in
[the evidence JSON](GENUI_AUDIT_2026-10-21_EVIDENCE.json). Source fingerprints and
installed versions are in [the snapshot](GENUI_AUDIT_2026-10-21_SNAPSHOT.json).
The initial parallel Rust log launch raced directory creation and did not execute;
the recorded Rust/Clippy results are from the subsequent actual executions.

The earlier report's 767 Rust passes, native chart and restart checks describe its
previous checkpoint. Likewise, the previous count of 89 Clippy diagnostics cannot
be presented as the current complete lint result: compilation now stops earlier.
The audit does not attribute the missing declarations/files to a particular author.

## What is actually implemented

### Architecture and modes

- Existing React/Tauri application is retained; no separate Next.js application or
  permanent Node inference/prompt server was introduced.
- `presentation.rs` provides deterministic Auto/Text/Visual routing, short static
  library profiles, final context checks and tool-free streamed formatting.
- `chat.rs:4197–4212` saves the authoritative agent answer/history, then invokes the
  formatter. Planning, structured tools, research finalization and control flow
  are not intentionally fed into the OpenUI renderer.
- Composer mode selection works on the first message and existing conversations.
  `usePresentationMode` protects conversation ownership during async saves; Chat
  holds submission while a conversation-mode save is pending.
- Separate application default, conversation override and per-message format exist.
- Text and ordinary Auto messages avoid the extra formatting inference. Visual
  messages currently always use a post-agent pass; there is no direct one-pass
  visual generation path. That is an explicit architecture tradeoff, not an
  automatically missing mandatory feature: the original request made a second
  strategy conditional on justification.
- Auto considers keywords and output reserve. It does not yet consider measured
  model UI reliability or actual available structured facts/cost beyond context.

### Official renderer, schemas and security

- Installed/pinned OpenUI runtime/core are 1.0.1, CLI 0.6.0, Zod 4.6.5. The broader
  `react-ui` package is not installed; a curated LocalLM library is used instead.
- Official `Renderer`, `defineComponent`, `createLibrary`, `.ref` relationships and
  `useStateField` are genuinely used, rather than screenshot demos.
- Official CLI generates full/core/data/education/coding prompt/spec pairs; parity
  tests compare generated artifacts with the actual library. Rust embeds assets.
- Validation bounds source, syntax, references, statements, nesting and arrays;
  rejects unknown components, cycles, dynamic expressions, queries and mutations;
  and validates materialized props with Zod.
- Bounded local repair handles named arguments, singleton arrays and a narrowly
  limited missing component closer. It does not invent defaults/factual values.
- No arbitrary JSX/eval/generated JavaScript or automatic code execution is exposed.
- Buttons allow visible follow-up messages and grounded credential-free HTTPS URLs;
  privileged action types are rejected. Follow-ups enter the normal permission
  controlled agent path. Existing tool/fence guards protect literal tool examples.
- CSP remains restrictive (`object-src 'none'; frame-src 'none'` etc.); Tauri
  capabilities were not weakened by GenUI. Provider adapters strip llama-specific
  fields before OpenAI-compatible transport; those fields are not a confirmed leak.
- Source IDs resolve against actual collected turn evidence. This proves available
  source identity, not truth of every generated title, summary or cited claim.

### Components and actual limits

| Group | Present | Limits/partial aspects |
| --- | --- | --- |
| Layout/text | Response, headings, text, Markdown, callouts, Section/Card, Stack/Row/Grid/Columns | Bounded layout nesting; no arbitrary React props |
| Data | Sortable Table, Stat, KeyValue, Progress, five SVG chart types and value legend | One point collection, no multi-series configuration; scatter defects below |
| Education | Diagram, Timeline, Quiz, Flashcard, Calculator, controls | Diagram is a numbered adjacency list, not a node/edge layout engine |
| Simulations | Insertion-sort-style swaps, binary search, LRU, BFS/DFS, SQL joins, toy gradient descent | Deterministic bounded examples; no arbitrary simulation code |
| Coding | Shared highlighted CodeBlock, FilePreview/search/collapse/line numbers, DiffViewer | FileTree is a flat path list; split diff is filtered text, not aligned hunks |
| Research | SourceCard, evidence-aware Markdown links, ordinary tables/timelines | No dedicated claim-audit/evidence-table/confidence UI integration |
| Interaction | Official local state, score/step/speed/rate controls, visible follow-ups | Standalone controls mostly display their own values; generic safe cross-component reactive bindings are absent |

Core has 17 components, Data 22, Education 27 and Coding 22 in the generated specs.
Core includes Diagram/Timeline/SourceCard, but excludes charts/code/actions/metrics.
Rich components share LocalLM tokens; charts use SVG, without overlapping engines.
The visual chunk is lazy, but specialized groups are eagerly imported inside it.

### Persistence and compatibility

- Additive tables: `message_presentations` and `conversation_presentation_modes`,
  both with cascading foreign keys. Global mode uses `presentation.default`.
- Original content remains in `messages`; raw program/state are separate and do not
  enter model history. Records include format/version/library/source/program/state/
  generation status/diagnostic. State writes enforce message/conversation ownership,
  completion and compatibility and have depth/size/prototype-key bounds.
- Restart marks partial visual metadata interrupted; malformed metadata can fall
  back without making the original answer unreadable. Answer edits invalidate UI.
- JSON export schema 2 includes presentation records separately; Markdown export
  retains the authoritative answer. Legacy Markdown/fenced conversations still work.
- Library mismatch falls back instead of silently reinterpreting a program.

## Prioritized findings and required fixes

### F01 — P0: the current Rust tree cannot compile

**Location:** `src-tauri/src/lib.rs:32,36,42,63,67,94,95,105,106`.
Declared but missing: `findings`, `formatter`, `instructions`, `permission_rules`,
`plugins`, `session_tools`, `snapshots`, `tasks`, `taste`.
**Impact:** native build, Rust tests and complete lint analysis cannot run.
**Action:** reconcile actual intended module wiring with source/index/history;
restore real implementations when required, or remove demonstrably orphaned
wiring. Do not create empty stubs or reset/stash/clean unrelated work. Run Rust
check/tests before declaring the older native binary valid. No fixes made here.

### F02 — P1: factual grounding does not preserve data associations

**Location:** `src/genui/dataGrounding.ts:9–24`, `VisualAnswer.tsx:61–69`.
For `Jan sales: 10; Feb sales: 15`, a chart with Jan=15 and Feb=10 passes both Zod
and the grounding function. It checks membership of each number and substring
presence of labels, not label/value relationships. Invented Table and Stat numbers
also pass: the guard examines Chart only. A FilePreview's contents are not checked
against the original code, though it is correctly labelled a sample.
**Impact:** syntactically valid visual output can contradict the authoritative
answer or show invented research/coding facts. This is not a tool-permission bypass.
**Action:** preserve typed evidence/fact associations, ground data-bearing components
and code/diffs against supplied authoritative structures, and fall back when facts
cannot be justified. Do not claim formal semantic proof for arbitrary prose. Add
swapped labels, changed units/values, invented rows/metrics/files and citation
association regressions. Suggested UI command: `$impeccable harden`.

### F03 — P1: valid early streaming content disappears on a partial child

**Location:** `validation.ts:110–119`, `ValidatedRenderer.tsx:17–26`,
`VisualAnswer.tsx:54–64`.
`root = Response([Text("First")` validates as streaming; appending
`, Chart("Sales","bar",[` makes the whole tree fail Zod on incomplete Chart props.
The wrapper replaces Renderer with a status paragraph, unmounting previously
rendered valid content. Model-produced named-argument programs remain invalid
through streaming and are repaired only after completion.
**Impact:** progressive presentation is inconsistent; entire content can disappear
and remount instead of remaining stable as additional components arrive.
**Action:** retain valid committed subtrees/last valid presentation while quarantining
incomplete nodes, preserve renderer identity and bounds, keep actions disabled and
raw syntax hidden during streaming. Test sequential prefixes of real Chart/Quiz
programs and measured named-argument outputs. Suggested: `$impeccable harden`.

### F04 — P1: power calculator silently changes a valid operand

**Location:** `Education.tsx:300–304`, `richCatalog.ts:180–182`.
`Calculator("c","Power","power",2,101)` passes the schema. The actual component,
rendered through the official Renderer in the audit probe, displays approximately
`1.26765060023e+30` (2^100), while 2^101 is `2.535301200456459e+30`. The exponent is
silently clamped to ±100 even though the input and schema permit larger values.
**Action:** compute the requested representable operation or explicitly reject an
out-of-policy operand in schema/control/output. Never silently calculate a different
problem. Cover exponents ±101, finite overflow and invalid domains. Suggested:
`$impeccable harden`.

### F05 — P1: native acceptance can exit successfully for a failed model case

**Location:** `scripts/native-genui-acceptance.py:143–161`.
A completed-or-error record satisfies the wait loop. Missing chart sets a false
flag, but the script writes JSON and returns success. Several fixture result flags
are set after actions without asserting the specific state transition. Restore-only
mode merges previous flags and carries no source/binary fingerprint.
**Impact:** a successful command is not necessarily a passing native model test;
an old executable or inherited results can be mistaken for current-source proof.
**Action:** explicit expected outcomes per scenario; required cases must assert
format/status/validated exact points/control state and fail on mismatch. Assert
simulator step independently of quiz state. Record fresh build/source hashes,
platform/model/config, timestamps and per-scenario results. Restore checks must
identify which checks are historical versus rerun. Retain intentional fallback
cases as separately successful *expected fallback* tests.

### F06 — P2: quiz radio groups collide across messages

**Location:** `Education.tsx:28`.
Radio names are `${p.id}-${i}`. Two independent messages can legally reuse a model
ID such as `joins`. Actual Quiz markup rendered twice in the probe creates names
`joins-0` in both; JSDOM native radio grouping unchecks the first message when the
second is checked. That leaves DOM selection inconsistent with per-message state.
**Action:** scope DOM group names to the owning message/unique component instance;
reject or disambiguate duplicate local state IDs within a program. Test two visible
messages with the same quiz ID, then save/reload both. Suggested: `$impeccable harden`.

### F07 — P2: global default changes do not refresh the mounted chat mode

**Location:** `PresentationDefaults.tsx:7–15,31–39`,
`usePresentationMode.ts:17–42`, `App.tsx:1099,1202`.
Settings is a modal over a still-mounted Chat. Saving the default changes only the
settings component/backend. The chat hook reloads only on conversation change, so
its “Default (auto)” label and preflight can stay stale while the backend actually
uses a newly selected Visual default. The settings component also lacks pending/
revision protection for an initial read versus a later save.
**Action:** share/invalidate the effective mode on default changes, guard async
settings responses, and test settings-save → close modal → same-chat preflight and
send. Do not overwrite model/tool preferences. Suggested: `$impeccable harden`.

### F08 — P2: scatter labels and accessible values omit/misplace x coordinates

**Location:** `Chart.tsx:85–99,127–135,154–175`.
For x values [0,1,100], actual point cx values are [40,45.2,560], but category labels
are positioned [40,300,560]. The legend contains Label/Value only and omits x.
**Impact:** the visual label does not identify its plotted point; screen readers
cannot recover both coordinates from the accessible values.
**Action:** use a numeric x axis/scaled labels and complete x/y values, with units and
an accessible data table. Cover all five chart types, zero/negative/constant ranges,
long labels and narrow containers. Suggested: `$impeccable adapt`.

### F09 — P2: renderer exceptions are not persisted as presentation failures

**Location:** `VisualAnswer.tsx:65–77,114–147`, `ValidatedRenderer.tsx:24–27`,
`VisualErrorBoundary.tsx`.
The new message path supplies neither official `onError` nor a boundary error
callback. It can show the original answer on an exception while stored generation
metadata and `presentation_end` still say complete. Parser/property rejection has a
host command, but render errors are not routed through it.
**Action:** bounded diagnostics and ownership-checked failure reporting for parser,
render and interrupted states; preserve original content and audit history. Keep
transport completion distinct from successful UI validation/rendering. Suggested:
`$impeccable harden`.

### F10 — P2: rich experiences are materially narrower than their names suggest

**Location:** `Diagram.tsx`, `RichComponents.tsx:42–49`, `Coding.tsx:84–104`,
`Education.tsx:251–294`, `specification.ts`.
Diagram is an adjacency list; FileTree is a flat list; split diff filters lines into
two unaligned pre blocks. Generic sliders do not drive arbitrary chart/math props.
Dedicated grounded evidence/claim-audit views, series toggles and richer mathematical
controls are absent. Existing toy simulations and basic tables are real and reusable.
**Action:** implement the specific original experiences with bounded configuration:
node/edge flow diagrams, hierarchical read-only file tree, parsed/aligned diff hunks,
typed local parameter bindings, and evidence/claim views backed by real harness
metadata. No generated JavaScript or automatic preview execution. Optional HTML/JS
preview and a second rendering design system are not required. Suggested:
`$impeccable shape`, `$impeccable harden`, then `$impeccable polish`.

### F11 — P2: performance and model-reliability evidence is incomplete

**Location:** `scripts/benchmark-{generative-ui.py,openui.mjs}`,
`docs/benchmarks/*`, `presentation.rs:89–157,307–322`.
The stored CPU benchmark is one sample per case, with different cache warmth; all
five raw visual programs needed repair. The coding case is supplied stdout, not
real tool-assisted generation. Current chunk sizes are not incremental baseline
bundle cost; validation timing is not paint or interaction latency. Native RAM,
first-visible-content, interaction percentiles, hardware/GPU and remote runs remain
unmeasured. Formatter token usage is not accumulated into normal inference rounds.
Auto routing has no model UI-reliability/capability gate. Visual mode always adds a
pass; a single-slot runtime may lose the agent KV cache.
**Action:** reproducible repeated measurements with configuration, cold/warm policy,
phase token counts, failures, validation/render results, actual tools and native
resource/interaction sampling. Profile real hot paths before virtualization or
heavy library changes. Add capability-aware graceful degradation. Evaluate direct
visual generation only if it can preserve final-answer/tool/history guarantees and
measured benefit justifies it. Suggested: `$impeccable optimize`.

### F12 — P2: acceptance coverage is incomplete, and one E2E wait is unsound

**Location:** `presentation_tests.rs` (five tests, one happy-path formatter SSE),
`e2e/generative-ui.spec.ts:16`, `src/genui/*.test.*`.
The browser persistence wait checks value != `'{}'`; initial `null` satisfies that
condition before the 250ms debounce saves anything. The suite has only three visual
E2E cases, with no visual malformed-output/error-boundary/stream interruption case.
Formatter cancellation, timeout, context downgrade, non-stop finish, provider
protocol variants and post-tool/research/approval/plan/resume integration lack
focused end-to-end coverage. Existing broad harness tests are valuable but not proof
of these new cross-feature combinations.
**Action:** wait for a specific non-null persisted state, add the missing scenarios
using existing mocks/native fixtures, and run authorized tests. Do not execute
Playwright or alternate localhost browser routes under the current AGENTS.md rule.
Document permanently unavailable platform checks rather than inventing passes.

### F13 — P3: some always-visible styles depend on loading the visual renderer

**Location:** `genui.css:254–255`, `library.tsx:11`, `MessageRenderer.tsx`.
The composer mode selector is app chrome, but its scoped styling lives in CSS
imported by the lazy visual library. Its styling can depend on whether visual code
has loaded. Move always-present composer styling into an eagerly loaded app/style
module, preserving tokens and scoped selectors; verify a cold Text-only session.
Suggested: `$impeccable polish`.

## UI audit health (bounded source-based assessment)

This is a desktop Tauri webview, not an iOS/Android app; mobile-native HIG scores
would be inappropriate. No browser preview was launched and no new native build
could be verified from the broken Rust source. Scores below are provisional source
assessments, not WCAG certification or new runtime conformance evidence.

| Dimension | Score / 4 | Evidence |
| --- | --- | --- |
| Accessibility | 2 | Native controls/labels and textual tables exist; radio isolation and scatter x alternative fail |
| Performance | 2 | Lazy renderer, compact tiers and stream throttling exist; whole-tree streaming rejection, duplicate parsing and full end-to-end benchmarks remain |
| Theming | 3 | Existing semantic tokens, container rules and previous light/dark native captures; cold selector CSS ownership needs cleanup |
| Responsive design | 2 | Scoped scrolling, wrapping and 480px containers exist; current native/text-scaling/keyboard runtime checks blocked, scatter labels incomplete |
| Implementation integrity | 1 | Real curated integration/security boundaries, but current native build is broken and generated factual/arithmetic correctness can fail |
| Total | 10 / 20 | Significant work needed; release verdict remains FAIL because F01 is blocking |

No new arbitrary font/color system was introduced. Existing DESIGN.md and its
previous prose/token drift were preserved. No sidecar or application code was
rewritten as an audit side effect.

## Original acceptance criteria: current status

| # | Requirement | Audit status |
| --- | --- | --- |
| 1 | Native app integration | Source implemented; current Rust build blocked by F01 |
| 2 | Text/Visual/Auto modes | Implemented including first turn; default synchronization defect F07 |
| 3 | Actual selected-model visuals | Historical real local GGUF chart evidence; current binary unverified |
| 4 | Validated curated OpenUI library | Implemented; schema validation is not factual validation (F02/F04) |
| 5 | Progressive streaming | Partial, whole-tree rejection/remount issue F03 |
| 6 | Correct charts and educational diagrams | Basic charts/simulations present; scatter and diagram scope gaps F08/F10 |
| 7 | Local interactions | Implemented for built-ins; radio isolation and generic bindings incomplete |
| 8 | Coding/research remain functional | Existing source paths retained; new combined integration coverage incomplete |
| 9 | Permissions cannot be bypassed | Strong source boundaries/allowlists; combined approval/host-action tests incomplete |
| 10 | SQLite persists UI/state | Implemented/tested at previous Rust checkpoint; latest Rust suite blocked |
| 11 | Old Markdown compatibility | Implemented and frontend tested |
| 12 | Invalid UI cannot crash chat | Parser guards/fallback/boundary implemented; render failure metadata incomplete |
| 13 | Fully local/no paid mandate | Implemented and historically exercised with actual GGUF |
| 14 | Baseline performance measured | Partial CPU/bundle samples only; full benchmark matrix missing |
| 15 | Automated tests pass | Frontend yes; Rust/lint currently fail, browser tests prohibited |
| 16 | Native behavior verified | Prior Linux chart/fixture/restart evidence only; current source blocked, other OS unverified |
| 17 | Documented/maintainable | Source map/usage/prompts exist; this audit supersedes prior release-status claims |

## Recommended implementation order

1. **F01 / P0:** restore a coherent native build, re-establish actual test/lint baseline.
2. **F02–F05 / P1:** fix grounding, incremental stability, arithmetic and acceptance truthfulness.
3. **F06–F09 / P2:** isolate controls, synchronize defaults, fix scatter and persist errors.
4. **F10 / P2:** implement bounded richer diagrams/coding/evidence/parameter experiences.
5. **F11–F12 / P2:** complete reproducible measurement and authorized regression matrix.
6. **F13 / P3:** `$impeccable polish` on the final tested implementation.

The user has explicitly requested a fresh implementation conversation, so the
[next-conversation brief](GENUI_IMPLEMENTATION_HANDOFF_2026-10-21.md) authorizes
normal implementation work without asking again between milestones. Browser and
platform restrictions persist. Re-audit resolved findings after changes; do not
replace this evidence with an unqualified “fully implemented” claim.
