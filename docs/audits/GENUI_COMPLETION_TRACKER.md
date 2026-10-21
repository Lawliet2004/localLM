# Generative UI implementation completion tracker — 21 October 2026

This ledger supersedes no historical evidence. The dirty working tree is preserved.
Browser/Playwright execution remains prohibited. Source changes detected concurrently
removed the composer mode selector; this implementation preserves that removal and
synchronizes the Settings preference with mounted chat/preflight.

| Finding | Implementation | Evidence / remaining gate |
|---|---|---|
| F01 | Removed orphan module declarations after checking callers/index/history; no stubs. A concurrent session subsequently supplied the real formatter module, which is preserved | Final live Rust check passed; pinned snapshot library suite: 821 passed, 17 ignored |
| F02 | Conservative clause/row association, exact code/diff/path provenance, owned citation association; host-bound SourceChart/SourceCode avoid factual transcription; ambiguous facts fall back | Audit reproductions in audit-regressions.test.tsx; no claim of semantic proof for prose |
| F03 | Commit complete component prefixes, quarantine unfinished children, normalize completed named calls, preserve Renderer identity | Sequential Chart/Quiz/named prefixes and node identity regression |
| F04 | Removed silent exponent clamp; finite requested result, explicit undefined/overflow states | ±101, overflow, fractional negative-base tests |
| F05 | Required native chart asserts exact points/schema/grounding/status and render control; quiz and simulator assert independent state; provenance and separate restore-only report | Fresh frozen-snapshot native chart/controls/restart passed; real tool case failed on runtime HTTP 500. Main harness correctly exited nonzero |
| F06 | React instance-scoped radio names; reject duplicate local state keys including implicit simulator/calculator keys | Two simultaneous quizzes and independent restoration tests |
| F07 | Settings publishes default changes; mounted chat refreshes without overwriting overrides; protect stale reads, serialize writes across modals, notify after modal closes | Hook and mounted-settings tests |
| F08 | Numeric scatter tick positions match plotted x; both coordinates in tooltip and data table | Scatter regression; native light/dark/narrow fixture review completed |
| F09 | Official renderer error and error boundary report owned rejection; bounded host diagnostic and separate rejection audit; transport success remains unverified rendering | Renderer exception test, SQLite ownership/idempotence test |
| F10 | Directed layered diagrams, hierarchical FileTree, aligned split hunks, typed Slider→FunctionPlot binding, local series toggles, host-backed EvidenceView | Experiences tests; no generated JS/execution authority |
| F11 | Formatter usage joins normal inference rounds with presentation phase; Auto backoff by exact model/endpoint; repeated rotated-case/cache-explicit CPU runner; native interaction/RAM samples | 18 repeated local samples retained including six failed visual outputs; compact transcription 0/3 grounded; bound table charts 3/3 raw/grounded/server-rendered. No remote/GPU measurements claimed |
| F12 | Rust transport truncation/tool rejection/cancellation/idle-timeout/budget/ownership tests; repair E2E persistence wait; add invalid/interrupted/renderer-error fixtures | Focused Rust 11 passed; plan/approval/write/final-answer/formatter integration passed; browser fixtures intentionally unexecuted; broader integration scenarios tracked below |
| F13 | Move composer styles into eager desktop CSS | Cold Text uses eager styles; composer selector removed concurrently |

## Final checks

Frontend: **784 passed, 1 skipped** (`npm test -- --maxWorkers=2`); typecheck passed.
Frozen-snapshot Tauri debug build passed. Presentation Rust tests: **11 passed**,
including real loopback Chat/Responses/Claude fixtures, cancellation, timeout and
successful Data→Core context downgrade. The plan/approval/write/final-answer/formatter
integration also passed. The full Rust library suite passed from a pinned test executable: **821 passed,
0 failed, 17 ignored**. The initial shared-target run had two `current_exe()` failures
after concurrent Cargo replacement; the complete pinned rerun omitted no tests.
Live `cargo check --lib` passed (14 warnings). Strict snapshot Clippy failed with
84 library / 102 test-library diagnostics; none target the presentation modules.
The two added diagnostics versus the previous check are in the concurrently restored
real formatter module. No lints were suppressed. Bin/doc tests were not rerun after
the initial full command stopped on the two shared-executable failures.

Native evidence: [delivery report](../benchmarks/genui-native-delivery-2026-10-21.json).
The selected CPU-only `gemma-4-E4B-it-qat-UD-Q2_K_XL.gguf` produced a grounded
SourceChart with exact 10/15/25 values and restored it in a separate restart run.
The real workspace-read scenario failed before a tool result: llama-server rejected
the prompt with HTTP 500, `NotImplemented: map: filter-mapping not implemented`.
The main native harness therefore **failed**, as required. No tool success is claimed.
Native proof applies to source SHA-256
`967a2fecaab847bd08c4380b74f41880bfef1d97f998ab0de73713e1881f6e49` and binary
`6b6c15018c8206b8f42006cbaef48258a38f4a3e1d8343e3bd17b9dc629d2bac`,
not subsequent concurrent unrelated edits.

Measured AT-SPI interaction: 20 samples, p50 43.96 ms, p95 49.88 ms; this includes
accessibility traversal/polling and is not paint latency. First persisted answer:
5.824 s; first accessible model chart: 21.890 s. Model-run high-water RSS per process:
app 244,012 KiB, WebKit renderer 236,884 KiB, runtime 3,089,304 KiB. These peaks
are not a simultaneous summed working set. Incremental visual JS/CSS totals:
275,156 bytes, 86,985 gzip bytes versus same-source Markdown substitutes.
The earlier 18-case experiment and failed transcription trials remain in the
benchmark reports; three successful bound-chart trials do not establish broad
model reliability.

## Acceptance criteria

1. Native integration: built and exercised in isolated Linux Xvfb/D-Bus against a pinned source/binary snapshot.
2. Modes: Settings Text/Visual/Auto, preserved backend conversation overrides and live mounted-chat synchronization. Concurrent composer-selector removal preserved.
3. Actual selected-model visuals: chart passed exact facts and restart; real tool-assisted acceptance **failed** on runtime prompt processing. Broad production acceptance remains open.
4. Curated library: seven generated official profiles with schema/prompt parity; typed source bindings and conservative association gates. Arbitrary prose semantics are not proven.
5. Progressive streaming: complete-subtree prefix commitment and stable interaction identity tested; unfinished children quarantined.
6. Charts/diagrams: numeric scatter axes, directed connections, data alternatives; native rich/light/dark/narrow fixture inspection completed.
7. Local interaction: isolated quizzes, exact saved state/restoration, safe slider defaults/bindings and common-scale series toggles tested.
8. Coding/research: shared code/diff primitives, literal source binding and host claim metadata implemented. Model tool-assisted integration remains failed above.
9. Permissions: tool-free formatter, malicious tool attempts rejected, owned state/rejection commands; no generated execution or host-action authority added.
10. SQLite: ownership, restart, cascade, interrupted programs and state bounds covered by Rust and native fixtures.
11. Legacy Markdown: original answer and history preserved; shared incumbent renderer retained.
12. Invalid output: parser/property/grounding/render failures retain authoritative text; malformed/interrupted native fallback verified.
13. Local inference: actual selected CPU GGUF chart succeeded; no paid provider needed for that proof. Selected model/runtime tool support failed.
14. Performance: repeated measurements retain failures; bundle, Node CPU, native interaction/RSS and inference timings documented with limits. No controlled production two-pass versus single-pass conclusion.
15. Automated tests: frontend/typecheck and focused Rust passed; full Rust library 821 passed / 17 ignored; strict Clippy failed as detailed above. Browser fixtures unexecuted by instruction; ignored tests are not passes.
16. Native behavior: isolated Linux evidence only. Windows/macOS, physical screen readers, remote/GPU and paid provider behavior unverified.
17. Documentation: usage/contracts, completion ledger, evidence and measurement boundaries updated; historical negative reports retained.

## Verification artifacts

- Consolidated machine-readable evidence: [implementation evidence](GENUI_IMPLEMENTATION_2026-10-21_EVIDENCE.json).
- Measurement scope and external gates: [verification boundaries](GENUI_VERIFICATION_LIMITS.md).
- Repeated model trials: [18 samples](../benchmarks/genui-repeated-local-2026-10-21.json), [failed transcription](../benchmarks/genui-compact-chart-negative-2026-10-21.json), [bound charts](../benchmarks/genui-bound-chart-2026-10-21.json).
- Native raw artifacts: `.local/qa/genui-delivery-native-2026-10-21/`; final source snapshot: `.local/qa/genui-final-source-snapshot-2026-10-21/`.
