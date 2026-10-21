# Generative UI implementation and verification

Current product decision (21 October 2026): the `Default (auto)` response-mode
selector and composer override editing are removed. Configure presentation in
Settings only; historical selector acceptance results do not authorize restoration.

> Current verification: [completion tracker](audits/GENUI_COMPLETION_TRACKER.md) and
> [evidence](audits/GENUI_IMPLEMENTATION_2026-10-21_EVIDENCE.json). The missing-module
> build blocker from the historical audit was repaired without stubs. Fresh native
> proof pins a frozen source snapshot because another session is editing this tree.
> The native chart and restart passed; the tool-assisted run failed on a runtime
> HTTP 500. Full production acceptance is not claimed. Earlier numbers below remain
> historical checkpoints.

This report supersedes the Milestone 1-only status in the historical foundation
report. Usage, schema migration, limits and generation architecture are documented
in [GENUI.md](GENUI.md). The original agent harness remains the authoritative
content/tool path. Visual presentation is an additional streamed, tool-free pass.

## Source map

| Area | Files |
| --- | --- |
| Final-answer formatter, deterministic policy, context checks, commands | `src-tauri/src/presentation.rs` |
| Additive SQLite migration, state ownership and restoration | `src-tauri/src/presentation_store.rs`, `presentation_tests.rs`, `store.rs` |
| Existing harness/channel/preflight integration | `src-tauri/src/chat.rs`, `lib.rs` |
| Separate presentation JSON export, original Markdown export | `src-tauri/src/export.rs` |
| Existing prompt/cache and fence-aware tool parsing compatibility | `src-tauri/src/prompt.rs`, `tool_calls.rs`, `harness.rs` |
| Frontend records, IPC and streaming state | `src/lib/types.ts`, `api.ts`, `src/App.tsx` |
| Settings defaults and read-only composer preflight | `src/components/Chat.tsx`, `Models.tsx`, `PresentationDefaults.tsx`, `src/hooks/usePresentationMode.ts` |
| Format dispatch and official Renderer | `src/genui/MessageRenderer.tsx`, `VisualAnswer.tsx`, `ValidatedRenderer.tsx`, `VisualErrorBoundary.tsx`, `VisualMarkdown.tsx` |
| Typed library and profiles | `src/genui/catalog.ts`, `richCatalog.ts`, `library.tsx`, `specification.ts`, `componentTypes.ts` |
| Validation, bounded syntax repair, actions and grounding | `src/genui/validation.ts`, `repair.ts`, `actions.ts`, `grounding.tsx`, `dataGrounding.ts`, `authoritativeData.ts`, `progressive.ts` |
| Reusable charts, diagrams, tables, controls, simulations and coding views | `src/genui/Chart.tsx`, `Diagram.tsx`, `DataTable.tsx`, `Education.tsx`, `Coding.tsx`, `RichComponents.tsx`, `Experiences.tsx`, `diff.ts`, `simulations.ts` |
| Shared code line numbers and scoped visuals | `src/components/CodeBlock.tsx`, `src/genui/genui.css` |
| Legacy fence/canvas compatibility | `src/components/MarkdownView.tsx`, `src/genui/GenUiBlock.tsx`, `GenUiSurface.tsx` |
| Generated schema/prompt assets | `src/genui/generated/*.{txt,spec.json}`, `src-tauri/src/genui_prompt.txt`, `scripts/build-genui-prompt.mjs` |
| Dependencies/build hooks | `package.json`, `package-lock.json` |
| Tests and benchmark/native tools | `src/genui/*.test.*`, `src/hooks/usePresentationMode.test.tsx`, `src/components/Chat.test.tsx`, `scripts/benchmark-{generative-ui.py,openui.mjs}`, `validate-openui-benchmark.mjs`, `native-genui-acceptance.{mjs,py}`, `e2e/generative-ui.spec.ts`, `e2e/fixtures/generative-ui-api.js` |

This is a focused implementation map, not a claim that every other dirty working
tree file belongs to this task. Existing uncommitted work was preserved.

## Automated verification

| Command | Result |
| --- | --- |
| `npm run typecheck` | Passed |
| `npm test -- --run` | 749 passed, 1 skipped; 108 passing test files |
| `npm run build` | Passed; existing large-chunk warning remains |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 767 passed, 17 ignored; no failures |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | Failed with 89 existing diagnostics; strict-lint gate remains open |
| `npx tauri build --debug --no-bundle` | Passed; embedded Linux app built |
| `python3 -m py_compile scripts/native-genui-acceptance.py scripts/benchmark-generative-ui.py` | Passed |
| `git diff --check` | Passed |
| Impeccable static detector on the task's UI targets | No findings |
| `npx playwright test` | Not run: workspace instructions explicitly prohibit browser localhost preview and alternate Playwright routes |

These results do not equate browser mocks with native verification. Rust tests
include an isolated loopback inference stream, SQLite ownership/cascade/recovery
and model-history separation. Frontend tests use the actual official Renderer,
including measured GGUF output fixtures, malformed programs, grounding and actions.

## Native acceptance

The harness uses the embedded Tauri/WebKit binary on an owned X11/D-Bus session,
with a separate database under `.local/qa/genui-native-verified`. It does not use
a browser preview, Vite, proxy, JavaScript injection or writes to production data.
The optional GGUF phase reads selected model/runtime preferences without changing
them and uses a capped context with GPU disabled in the isolated database.

```bash
LOCALLM_SMOKE_OUTPUT=.local/qa/genui-native-verified \
LOCALLM_MODEL_PREFS_DB=/path/to/read-only/source/locallm.sqlite \
node scripts/native-genui-acceptance.mjs
```

Requires Xvfb, accessibility D-Bus/AT-SPI, Python GI, ffmpeg, xwininfo, xclip
and libXtst for native clipboard paste on the isolated display. `LOCALLM_XVFB_BIN` can name an extracted
Xvfb binary. No system packages need be installed by the harness.

The deterministic native fixture checks quiz scoring, binary-search stepping,
SQLite state writes, app restart/restoration and unchanged authoritative text,
then captures desktop light/dark and a 760px-wide native window. The optional model
phase loads the actual configured GGUF, submits a visible sales question through
the composer, waits for the real presentation row and checks the chart control.
Its machine-readable result is `acceptance.json`; screenshots and raw presentation
metadata are stored alongside it. The [recorded Linux native result](benchmarks/openui-native-linux.json)
confirms all six fixture/state checks, actual selected-model chart rendering and
restoration of that model-generated chart after restarting the application. The
rebuilt native app also exposes the selector before the first message. The
three chart points are January 10, February 15 and March 25. Raw named arguments
needed local syntax repair. The native formatter event measured 14.003 seconds
for this one presentation pass (not whole-turn latency or first paint). No
production database was written.

The driver checks that the submitted prompt exactly matches the intended text.
WebKit exposes the select as “Chart type bar”; the matcher accepts the native
combo-box role plus that label prefix. Earlier exact-label checks reported false
negatives even when the chart was visible. Recheck an existing model chart without
another model request using `LOCALLM_RESTORE_VISUAL_ONLY=1` with the same output
path. [Earlier native attempts](benchmarks/openui-native-attempts.json) include
invalid outputs and safe fallback; these results are not a general success rate.

## Measured performance

[GGUF measurements](benchmarks/openui-gemma-cpu.json) use the configured
`gemma-4-E4B-it-qat-UD-Q2_K_XL.gguf`, CPU inference, 8,192 context and 512 output
tokens. They are isolated single samples, not full native turn latencies:

| Case | Prompt tokens | First token | Total generation |
| --- | ---: | ---: | ---: |
| Markdown baseline | 7 | 1.042 s | 5.612 s |
| Core | 1,375 | 4.373 s | 9.873 s |
| Data | 1,649 | 5.006 s | 10.820 s |
| Educational interaction | 1,895 | 5.727 s | 12.314 s |
| Quiz | 1,901 | 0.757 s | 6.953 s |
| Coding execution-result fixture | 1,631 | 5.443 s | 10.781 s |

All five raw visual programs failed syntax validation. All five passed after the
bounded local syntax repair and rendered in official Renderer tests. This is not
a measured general success probability. The coding case presents supplied stdout;
it does not measure a tool execution. Runtime peak RSS rose from roughly 3.17 GiB
to 3.51 GiB across sequential cases; this includes model/runtime allocation and
cannot isolate renderer cost. Cache warmth differs by case. These timings predate
the final catalog additions; current prompt byte sizes are recorded separately.

[Final frontend CPU/bundle measurements](benchmarks/openui-production-overhead.json)
record warm validation p95 around 0.19–0.36 ms for the small supplied programs.
Current lazy visual chunks total approximately 236 KiB raw / 74 KiB gzip, excluding
CSS and shared chunks already used elsewhere. These are current chunk sizes,
**not** a measured incremental baseline. Profile prompt sizes and Node RSS are in
the JSON. Node RSS is not native-app RAM. First-token timing is not first painted
UI. Native paint latency, interaction latency percentiles, GPU runs, full native
RAM comparison and remote-provider performance remain unmeasured.

Reproduce the isolated benchmark with:

```bash
python3 scripts/benchmark-generative-ui.py \
  --preferences-db /path/to/locallm.sqlite --output /tmp/openui-benchmark
node scripts/validate-openui-benchmark.mjs /tmp/openui-benchmark/results.json
node scripts/benchmark-openui.mjs > /tmp/openui-frontend.json
```

## Remaining verification limits

Strict Clippy and browser E2E acceptance remain unpassed. Ignored tests are not
reported as passing. Live remote-provider visual generation and non-Linux native
platforms have not been separately verified. A second visual inference pass adds
latency and can displace a single-slot KV cache. Generated prose is constrained by
instructions and evidence checks, not a formal proof of semantic fidelity. The
renderer has no arbitrary HTML/JS preview or generated host-command execution.

## Audit remediation update

See [the completion tracker](audits/GENUI_COMPLETION_TRACKER.md) for current checks;
historical pass counts and old native captures are not current-source acceptance.
The hardening adds conservative factual provenance, committed streaming prefixes,
scoped controls, settings synchronization, exact arithmetic, numeric scatter axes,
owned render rejection, richer bounded experiences and formatter metrics/backoff.
`docs/GENUI.md` documents the added component contracts and additive state semantics.

The benchmark runner now rotates cases over repeated samples with an explicit cache
policy and source/runtime/model fingerprints. Its actual Python process case is real
fixed orchestration, not model-selected tool use. Native acceptance asserts exact
chart associations, independent quiz/simulator persistence and restart state, and
records fresh source/binary provenance and AT-SPI action-to-state latency. Restore-only
runs write a separate report and never inherit historical pass flags as rerun proof.
Browser fixtures include invalid/interrupted/render-error fallbacks and wait for the
specific persisted quiz selection; they remain unexecuted under AGENTS.md.
