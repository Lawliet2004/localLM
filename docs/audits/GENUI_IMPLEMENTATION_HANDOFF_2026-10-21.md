# Fresh conversation: finish LocalLM generative UI

> Superseded product requirement (21 October 2026): the user removed the
> `Default (auto)` response-mode selector and composer override editing.
> References below to implementing, styling or validating that selector are
> historical and must not be followed. Presentation preferences are Settings-only.
> See `AGENTS.md` and `DESIGN.md`.


## User authorization and objective

The user explicitly requested: “thoroughly audit what things are in there and
start a new conversation to implement the rest of the things.” This is the
implementation conversation, not another planning-only or audit-only task.
Implement the remaining independently achievable work, persist through normal
milestones without asking for confirmation, and leave unavailable checks honestly
unverified. Do not claim full production acceptance until the actual gates pass.

Working directory: `/home/papan/Desktop/Projects/localLM`.
Repository: https://github.com/Lawliet2004/localLM .
Read `AGENTS.md` first, then these audit artifacts:

1. `docs/audits/GENUI_AUDIT_2026-10-21.md` — verified inventory, F01–F13 findings,
   exact current checks and all 17 acceptance criteria.
2. `docs/audits/GENUI_AUDIT_2026-10-21_EVIDENCE.json` — executable probe outputs and
   exact programs that reveal grounding/arithmetic/streaming/radio/scatter defects.
3. `docs/audits/GENUI_AUDIT_2026-10-21_SNAPSHOT.json` — observed source hashes,
   installed versions, build/test results and missing-module list.
4. `docs/GENUI.md`, `docs/GENERATIVE_UI_IMPLEMENTATION.md`,
   `docs/OPENUI_ARCHITECTURE_AUDIT.md` — implementation/history, not current proof.
5. `docs/RENDERING.md`, existing `PRODUCT.md`, `DESIGN.md` and applicable skills —
   preserve the established desktop UI and semantic tokens.

This task inherits the current working directory, not a clean commit. HEAD at
audit was `5eb2ba3d135725dc0a9bc6fb7156bf983e25d718`, with 434 dirty entries before
audit documents. **Do not reset, stash, clean, overwrite unrelated work or create
empty stubs to make compilation appear successful.** Re-read changed files if
fingerprints differ; the shared workspace may contain concurrent work.

## Non-negotiable architecture

This is the existing Tauri 2 / Rust / React 19 / TypeScript / Vite app, using
llama.cpp GGUF and remote providers, SQLite, agent tools, MCP, SearXNG, prompt
caching, preflight and permission auditing. Keep those systems intact.

Preserve the boundary:

`composer -> presentation policy -> existing agent/tools/research -> authoritative
answer/history -> tool-free presentation -> validation -> official Renderer ->
local state or visible normal follow-up -> existing tool authorization`.

Never parse planning, tool-call arguments/results, AREX/control/finish or reasoning
as a privileged UI. No generated JS/JSX/eval/Function, arbitrary Tauri commands,
automatic code execution, hidden follow-up turns, or weakened CSP/capabilities.
Existing tool envelope, artifact capture, native history replay, MCP ownership and
workspace scoping conventions remain authoritative. External content is data.

Original answer, raw program, rendered state, model-visible history and tool audit
must stay separate. Keep old Markdown conversations readable. Record actual format
per message, independent of the current mode. Continue using official OpenUI
Renderer and typed curated schemas rather than inventing a parallel UI language.

Installed runtime/core: `@openuidev/react-lang` and `@openuidev/lang-core` 1.0.1;
CLI 0.6.0; Zod 4.6.5. Inspect installed declarations before changes. `react-ui` was
evaluated but is not required. No new permanent Node server, second app/design
system, multiple chart engines or heavyweight editor is needed.

Official references (check current APIs before introducing new SDK behavior):

- https://github.com/thesysdev/openui
- https://github.com/thesysdev/open-intelligent-ui
- https://www.openui.com/docs/openui-lang
- https://www.openui.com/docs/openui-lang/renderer
- https://www.openui.com/docs/openui-lang/system-prompts
- https://www.openui.com/docs/openui-lang/message-protocol
- https://www.openui.com/docs/openui-lang/interactivity
- https://www.openui.com/docs/openui-lang/reliability

## Start here: F01 build blocker

Fresh audit TypeScript checks, 749 frontend tests and frontend build pass. Rust
**does not compile**: `src-tauri/src/lib.rs` declares nine absent files/modules:
`findings`, `formatter`, `instructions`, `permission_rules`, `plugins`,
`session_tools`, `snapshots`, `tasks`, `taste`.

Reconcile these with actual source, staged changes and history. Preserve real
features and intended work; do not assume a clean checkout or fabricate placeholder
modules. Once coherent, run `cargo check --lib`, actual Rust tests and Clippy to
find the *current* baseline. The prior “767 Rust passes/89 Clippy diagnostics” was
a historical checkpoint; neither count describes the current stopped compilation.
An old native executable still exists and must not be used as proof of a fresh
build. Current lint sees missing modules and partial/cascading diagnostics.

## Implement in this order

Use the audit's finding IDs in a completion ledger (new
`docs/audits/GENUI_COMPLETION_TRACKER.md`). Mark each implemented/tested, partial,
blocked or deliberately deferred with evidence; distinguish mandatory features
from optional optimizations. Begin real code changes after the initial source
recheck, not another long audit.

### 1. Correctness and trustworthy acceptance (F02–F05)

- Ground label/value/unit associations, not unordered number membership. Reproduce
  Jan=10/Feb=15 becoming Jan=15/Feb=10 and invented table/metric rows. Preserve
  original typed data/evidence/code where available; avoid fabricating facts or
  changing literal code/diffs during formatting. Where provenance is insufficient,
  fall back to original Markdown rather than pretending semantic proof.
- Make incremental rendering keep already validated visible content when another
  child is incomplete. Preserve Renderer identity/local state. Quarantine invalid
  nodes, maintain resource limits, disable interactions during generation and never
  show raw syntax as interim prose. Include root-first references and measured
  named-argument output variants; do not blindly repair unfinished strings/EOF.
- Fix Calculator: 2^101 currently displays 2^100 because a valid exponent is silently
  clamped. Explicitly validate/reject unsupported operands or compute correctly;
  handle division by zero, negative fractional powers and finite overflow clearly.
- Make native acceptance assert expected success/fallback outcomes and exit nonzero
  on failed required scenarios. A complete error record or missing chart currently
  can return success. Record exact build/source/model/config/time provenance, exact
  chart points and independent step/state assertions. Avoid inheriting old pass
  flags as if rerun on a new binary.

### 2. Interaction isolation, settings and diagnostics (F06–F09)

- Namespace radio group names by message/component instance; prevent duplicate
  control IDs within a program from silently sharing state. Test two messages both
  using Quiz id `joins` and persist/reload each independently.
- Refresh mounted Chat when application default changes in the settings modal.
  Align displayed default, preflight and actual send; protect initial settings read
  against later saves and out-of-order responses without a duplicate state system.
- Correct scatter numeric x axis/labels and accessible x/y table/tooltip. Test all
  supported chart types, negative/constant data, long labels and narrow containers.
- Connect official onError and render boundary failures to bounded diagnostics and
  ownership-checked persistence/audit reporting. Keep model transport completion
  distinct from validated/rendered UI success, and preserve the authoritative text.

### 3. Finish the requested richer experiences (F10)

Existing implementations are real but deliberately basic. Improve them through
bounded reusable components, not hard-coded responses:

- Flow/process/concept/decision diagrams should have a meaningful node/edge layout
  plus an accessible textual alternative. Current Diagram is an adjacency list.
- Read-only FileTree should present hierarchy/expand-collapse; no filesystem tool
  authority. Split diff needs correct hunk alignment/gaps, not filtering lines into
  independent pre blocks. Reuse existing highlighting, Markdown and diff primitives.
- Add a small safe typed parameter-binding mechanism where controls should drive
  visual/math data beyond their own displayed value. Built-in sorting, binary
  search, LRU, BFS/DFS, SQL joins and toy gradient descent already exist; keep them.
  Do not enable arbitrary expression execution merely to provide reactivity.
- Render research evidence/claim-audit details, grounded comparisons and source
  metadata using actual existing harness structures. Unknown IDs must not become
  fake citations; generated titles/summaries are not automatically source truth.
- Series toggles/comparisons and mathematical controls should remain local where
  feasible, with bounded data and accessible labels. Preserve responsive themes.

An isolated HTML/JS preview is optional; skipping it is preferable to adding an
unsafe execution environment. A direct single-pass visual strategy is also
conditional: evaluate only if measurements show benefit and final-answer/history/
permission/fallback guarantees can be maintained. Do not route every message through
another classifier or formatter. Ordinary Text and simple Auto must stay fast.

### 4. Measurement, integration coverage and final polish (F11–F13)

- Record baseline Markdown vs Core/Data/Education/real tool-assisted UI using repeated
  runs, documented cold/warm conditions and actual selected local models. Include
  prompt/formatter tokens, first visible content, full response time, validation and
  render success (raw and repaired), peak process/native RAM, context utilization,
  incremental bundle size and interaction latency percentiles. Count failures.
- Existing CPU JSON is single-sample and its coding stdout is a fixture, not actual
  tool execution. Current chunk size is not incremental baseline; parser CPU is not
  first paint; Node RSS is not native memory. Keep earlier failures in the record.
- Make Auto degrade with actual capabilities/reliability/output/context budget. Do
  not raise user memory/context limits. Keep frozen agent prompts/cache ordering
  stable; document single-slot cache displacement and any explicit invalidation.
- Add focused Rust/provider-stream tests for cancellation, timeout, context
  downgrade, truncation/non-stop finish and protocol variants. Add tool/research/
  approval/plan/pause/resume combinations around the final presentation boundary.
- Cover old Markdown, mode/model switching, interactive reload/restart, library
  mismatch, malformed metadata, edits, deletion/cascade, exports and large payloads.
- Repair the E2E persistence wait: initial null already satisfies != `'{}'`. Assert
  actual non-null expected state. Add error-boundary/invalid/stream interruption
  cases, but obey the browser prohibition below.
- Move always-visible composer mode styles out of lazy renderer CSS. Profile before
  adding virtualization or new dependencies; preserve shared tokens and reduced
  motion. Finish with a bounded design/accessibility/responsive review and docs.

## Required scenario checklist

1. DNS explanation → ordinary concise text in Auto; zero unnecessary formatter call.
2. TCP handshake → accurate SYN → SYN-ACK → ACK diagram.
3. Sales January10/February15/March25 → exact supplied associations, no swaps/invention.
4. Binary search → correct local next/previous/reset steps and restoration.
5. Python LRU answer → highlighted copyable code/diff when relevant; no auto execution.
6. Relational vs graph DB research → actual evidence/citations/claim details preserved.
7. SQL joins quiz → scoring, independent message ownership, state saved/restored.
8. Malformed/unknown/cyclic/oversized program → non-crashing original-answer fallback.
9. Shell-execution button/action → no direct execution; ordinary host permission path only.
10. Cancel mid-program → interrupted presentation, stable original answer/audit, no false
    rendered-success status or pending expensive work.

Also test default/override transitions, switching model providers, cache stability
and low context/output budget. A mock proves only its exercised layer; distinguish
it from native persistence/inference evidence.

## Verification and environment rules

Run appropriate focused checks while working, then required final commands:

```bash
npm run generate:openui
npm run typecheck
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npx tauri build --debug --no-bundle
```

**Browser localhost preview was explicitly declined. AGENTS.md forbids bypassing
this through alternate browser/proxy/Playwright routes. Do not run Playwright or a
browser preview under these instructions.** Add/repair fixtures and report that
check unexecuted until the user changes the restriction. Do not ask for permission
between normal milestones.

Embedded native Tauri on an owned Xvfb/D-Bus display is a separate authorized route.
Existing `scripts/native-genui-acceptance.{mjs,py}` uses isolated XDG directories,
SQLite, AT-SPI, xclip/libXtst, ffmpeg and optionally a read-only copy of selected
model preferences. It never writes the user's DB. Xvfb was previously extracted
under `/tmp/locallm-native-deps/root` without system installation; check availability
before reuse. Existing verified output was `.local/qa/genui-native-verified`, but
start a fresh run linked to the newly built binary and fix harness assertions first.
Do not treat current old binaries or merged restore flags as a new passing run.

TCP restrictions were observed in prior sandboxed environments, not a universal
network ban; run real loopback tests only where allowed and distinguish failures.
Never claim skipped/ignored/platform-blocked tests passed. Do not expose credentials
or copy production settings into committed benchmark files. Remote paid/provider
verification requires actual available authorization/configuration; don't invent it.

## Completion deliverables

- Actual code fixes and reusable rich components, with generated prompt/schema parity.
- Backward-compatible versioned migration/state semantics if schemas change.
- Updated completion ledger keyed to F01–F13 and all 17 original acceptance criteria.
- Focused regression tests for each concrete audit reproduction.
- Actual final command results and fresh native evidence tied to exact source/build.
- Measured performance/reliability with failures and limitations, not arbitrary claims.
- Updated usage, component, prompt, streaming, persistence and security documentation.
- Concise final list of exact changed files, verification and remaining external gaps.

Preserve existing successful architecture. Do not stop after another foundation,
plan or token-limited partial result while independently achievable work remains.
