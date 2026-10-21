# LocalLM review completion audit

User objective: open a new chat and fix all recommendations from the October 17
LocalLM review. Implementation chat: `01a14985-f1ce-7a42-a983-4d76ddd7ba6f`.

This is the parent task's final acceptance inventory. Current verification is summarized in the table; the chronological observations retain earlier failures and their resolutions.
The implementation task also maintains its own working checklist. Evidence refers
to current source, actual command results, runtime observations, or inspected
captures. Historical test counts and asserted intentions do not close an item.

Source reviews:

- `.impeccable/critique/2026-10-17T07-23-15Z__src-components-chat-tsx.md`
- `.impeccable/review/swe2-max-research-2026-10-17.md`
- The parent conversation's final recommendations (including capability badges,
  command palette, latency diagnostics, project indexing, and voice).

## Requirements

Current evidence is summarized below. Platform and provider boundaries are stated explicitly; they are not represented as live verification.

| ID | Requirement | Acceptance evidence | State |
| --- | --- | --- | --- |
| R01 | Separate chat created and implementation started | Thread creation result and live thread status | Done: thread ID above |
| R02 | Persistent, validated Auto/Quick/Think reasoning preference | Native reasoning preference regressions (old preferences default Auto; invalid values rejected); saved preferences and per-response override in App/Chat | Verified in current broad frontend/native gates |
| R03 | Capability-aware local reasoning controls on ordinary requests | Pinned b11669 /apply-template probe; built-in Gemma controls detected through /props; real native Quick answer 4, capability enabled, no reasoning in native-runtime-acceptance.json | Verified for installed runtime/model; unknown templates remain unavailable |
| R04 | Provider-aware remote reasoning controls | Backend::payload and Claude adapter regressions capture supported/unknown/subscription transformations; local fields stripped, compatible token budget used | Verified adapter payloads; live external APIs unavailable without configured credentials |
| R05 | Direct-answer behavior and smaller relevant tool catalogs | Catalog selection freezes once per turn; Quick omits optional orchestration/read helpers, retains search/weather/explicit tools and recall keywords; native boundary regressions | Verified; no general speedup claim |
| R06 | Separate answer-length control | Inference retains separate max response-token budget beside Auto/Quick/Think; native ordinary-mode regression asserts output budget unchanged | Verified |
| R07 | Honest timing and no fabricated duration | Real native persisted response metrics; legacy serde regression and ResponseMetrics unavailable states; fabricated 8s/word-count estimates removed | Verified |
| R08 | First visible answer and phase timing | Native response records preparation, model rounds, first visible answer and total; monotonic tool/approval accumulation inspected in chat.rs | Verified native dispatch/answer boundary and all phase persistence/fork; tool/approval timers inspected, no live forced-approval timing claim |
| R09 | Answer-first conversation with stable reasoning placement | ActivityTimeline stable-disclosure and fallback regressions; browser acceptance answer-first/collapsed state passes; final capture batch passed | Verified in final browser acceptance and coordinated UI confirmation |
| R10 | Readable markdown, code and contextual actions | Browser populated answer/table/code and copy actions pass; single code frame repair included in final capture batch | Verified in final browser acceptance and coordinated UI confirmation |
| R11 | Typed tool results and concise activity | Typed web/command/file/diff/weather/artifact renderers; pending/errors outside collapsed work; failed structured result overrides exit zero, independent browser regression passes | Verified |
| R12 | Sources attached to answers | AnswerSources metadata/dedup tests, safe source utility and citation plugin, full-result hydration scoped by conversation; independent browser answer and expanded-tool checks pass | Verified |
| R13 | Search progress, partial results and explicit depth | Existing worker phase/progress/depth controls preserved; fast/normal planner skipping, partial source failures and recovery covered by current web tests | Verified in current broad frontend/native gates |
| R14 | Shared artifact viewer | Shared ArtifactViewer text/image/Markdown/code/CSV/TSV Preview/Source; HTML literal; browser native-transport action tests and actual WebKit Open/Save acceptance | Verified |
| R15 | Artifact overflow and accessibility | 17-output browser case reaches last artifact, distinct accessible actions and close; native saved CSV matches source bytes/SHA-256 | Verified |
| R16 | Consolidated settings taxonomy | SettingsNavigation implements six groups with language/weather child entries and searchable destinations; final desktop/narrow settings captures passed | Verified in final browser acceptance and coordinated UI confirmation |
| R17 | Unified trace/theme tokens | Trace tokens derive from shared accent; named ANSI theme palette and readable dim output; final light/dark/all-five-accent computed checks passed | Verified final light/dark/all-five-accent token checks; no contrast candidates in reviewed scenes |
| R18 | Command palette extending shortcuts | CtrlShiftP/menu palette, action/navigation/focus restore pass; short-window selection scroll and 200% browser zoom keyboard path verified | Verified |
| R19 | Model capability indicators | Compact capabilities disclosure reflects local/API path and reasoning/tools/images; unsupported/unknown reasoning disabled; native Gemma capability observed enabled | Verified final capability disclosure layout and actual native capability state |
| R20 | Repeatable latency diagnostic | Standalone runner protocol/cleanup tests; actual Quick simple/code/research tool/cache matrix and Auto/Think arithmetic; real in-app native diagnostic 6 streams, 4/4 measured correct | Verified; model citation failures retained, small samples establish no speedup |
| R21 | Accessible image modal | Real browser image keyboard/modal/Escape/focus acceptance passes; background app stays inert | Verified |
| R22 | Accessible result/trajectory selection and status | Result/trajectory filters use valid pressed buttons; per-turn concise status; keyboard image/palette paths verified | Verified in current broad frontend/native gates |
| R23 | Recoverable conversation-search failure | Independent browser failure retains search query and retries; Sidebar polite atomic status reports searching/results/count | Verified |
| R24 | Responsive, zoom and reduced-motion behavior | True browser setZoom/getZoom 2 verified in isolated Chromium: viewport1440x1000→720x500, DPR1→2, CSSzoom1; no overflow, palette keyboard; final light/dark narrow contrast/motion batch passed | Verified bounded desktop/narrow/light/dark/motion/actual 200% zoom acceptance |
| R25 | Bounded activity rendering work | Indexed joins and binary-search per-turn partitioning; memoized Markdown regression; replayable paired benchmark identical assignments up to 2,000 turns / 10,000 events | Verified event-partition improvement only; not inference/paint speedup |
| R26 | Remove redundant UI/render paths | Legacy TurnTools removed; one timeline, optional diagnostics/raw details, code wrapper repair, unified 64-tool cap with actual preflight count and overflow blocking | Verified final UI/functional and broad frontend/native gates |
| R27 | Opt-in project-document indexing | Native backend tests plus actual desktop opt-in/search/edited-file/deletion/disable workflow in native-document-acceptance.json; bounded 16 × 32 KiB selected local files, citations/untrusted excerpts and API disclosure | Verified bounded keyword retrieval; no background embedding scan |
| R28 | Voice input | Browser service explicit start/transcript-to-draft, error/unavailable/cancel; independent mocked speech tests include conversation switch and suppression of late results | Verified capability/lifecycle; physical microphone/service not exercised |
| R29 | Preserve existing workflows | Final combined 30/30 browser acceptance/workflow cases, including restored independent Stop; frontend/native broad workflow tests | Verified 30/30 browser workflows and current broad frontend/native gates |
| R30 | Current build and regression gates | Current production build/TypeScript passes; frontend 529 passed / 1 skipped, native 449 passed / 5 ignored; cargo clippy --all-targets -- -D warnings passes; browser 30/30 and UI 8/8 | Verified current build, frontend, native, strictClippy and browser gates |
| R31 | Final evidence and cleanup | Current README and audit cover new features/boundaries; research, benchmark, native/browser evidence saved; parent servers/browser contexts stopped; no remaining marked review processes | Verified implementation evidence and owned cleanup; final implementation summary linked below |

## Completion rule

Do not mark the parent goal complete while any item is open, contradicted,
unsupported by current evidence, or awaiting a required external verification.
Classify external blockers explicitly; implementation alone does not establish
runtime acceptance. Preserve all pre-existing uncommitted changes.

## Independent observations

2026-10-17: the parent executed the installed
`/home/papan/.local/share/app.locallm.desktop/runtimes/llama-b11669-cuda12.8-linux-x64/llama-server --help`.
It exited successfully and advertises `--reasoning [on|off|auto]`,
`--reasoning-effort LEVEL`, `--reasoning-budget N` (`-1` unrestricted, `0`
immediate end), and `--chat-template-kwargs`. This verifies flag availability
for R03 only. No inference server was started by this check and actual
template-less-model behavior remains open. `--reasoning-format none` leaves
thoughts unparsed; it is not a thinking-disable control.

Current source inspection also confirms the removal of literal `8s` and
word-count estimates in `ActivityTimeline.tsx`, and indexed result/run/message
joins. R07/R25 remain open pending complete timing, performance and UI acceptance.

The parent then ran an isolated CPU-only Gemma probe with the installed runtime
(2 threads, 2048 context, Q8 KV, no projector or GPU layers). The replayable
probe and results are in `.impeccable/review/runtime-reasoning-probe/`.
`measurements.json` records successful startup, four completed SSE requests and
`processStopped: true`; the temporary runner was removed.

Important correction to the original research: the model's built-in runtime
template **does** honor per-request `chat_template_kwargs.enable_thinking`.
`/apply-template` with `false` removes the `<|think|>` system marker and changes
the synthetic input from 35 to 33 tokens; default/true include it. GGUF-only
detection would miss this capability. Runtime `/props` reports tool support
and `supports_reasoning_effort: false`; that latter field must not be confused
with lack of an enable-thinking switch.

All four deterministic `2 + 2` requests answered `4` with no observed reasoning,
including default, template-off, template-on, and request `reasoning_budget: 0`.
First visible answer was about 2.26–2.54 seconds in this CPU configuration.
This tiny sequential sample is **not** a statistical speedup or evidence of
long reasoning on this prompt. Acceptance of the budget parameter alone does
not establish budget enforcement. R03 still needs app-integrated controls and
regressions; R20 still needs its complete diagnostic matrix.

The parent implemented `scripts/benchmark-response-latency.py`, its focused
regressions, and `docs/RESPONSE_LATENCY_BENCHMARK.md`. Ten protocol/correctness/
aggregation/process-cleanup tests pass. The runner creates only isolated local
requests and a bounded synthetic fixture tool; it never executes generated
code or contacts a search service. It exposes median/nearest-rank p95 with
sample counts and keeps correctness failures visible.

Live evidence:

- `.impeccable/review/response-diagnostic-quick-cpu/measurements.json`:
  18 completed streams (12 measured cases, six primes), 10/12 measured fixture
  passes. Research with tools enabled omitted required `[A][B]` citations in
  cold/prime/warm cases; the runner correctly returns failure. Do not weaken
  correctness checks to convert this into a passing model result.
- `.impeccable/review/response-diagnostic-auto-think-cpu/measurements.json`:
  12 completed arithmetic streams, eight measured cases, all correct.
- Both reports record `processStopped: true`; all parent-owned servers exited.
- Warm-request backend cache counters confirm actual prompt reuse (34–131
  cached tokens across the Quick task/tool groups; 36–96 for Auto/Think).

R20 has a repeatable standalone backend diagnostic and actual native evidence.
App-integrated phases/approvals, real search/tool quality, user-facing diagnostic
integration and a complete app comparison remain open. Initial reports used one
sample per group; their p95 equals the single observation, not a distribution
estimate. Default future runs collect three samples. No application speedup is
claimed from these standalone observations.

Independent browser functional acceptance now lives in
`e2e/review-acceptance.spec.ts` with a native-transport fixture extension.
Screenshots are disabled to preserve the implementation task's coordinated
visual review rounds. Current passing coverage: completed reasoning/activity
collapsed with no invented duration; readable answer/table/code; deduplicated
visible answer sources; keyboard image modal with Escape/focus restore;
palette normal navigation/action/Escape; search failure/query preservation/
retry; 17-file artifact overflow; HTML source nonexecution; actual file
download; artifact close; CSV Preview/Source. Three speech-service-mock tests
also pass: explicit start and transcript review without send, cancellation,
and unsupported-webview messaging. These do not establish physical microphone
or transcription-service acceptance on any OS.

The independent tests exposed two still-open implementation defects and the
implementation chat has reproductions: the palette's selected last option
remains off-screen at 900x480, and artifact Open uses a blocked top-level data
URL. Initial test-only locator/native-browser-focus assumptions were corrected;
they are not product defects. R18/R14 remain open until the real gaps pass.

Both browser defects have now been repaired and their two focused acceptance
tests pass: keyboard selection scrolls into view, and Open displays readable
text via a lifecycle-managed Blob URL. Native artifact Open/Save remains to be
verified: the installed Tauri opener plugin intercepts external HTTP(S) links
but not Blob URLs, so Chrome popup success is not native-webview evidence.

Further source review found diagnostic acceptance gaps, reported to the
implementation task: imported incomplete rows enter latency aggregates; the
displayed median is nearest-rank p50 even for even-sized imports; native
`[DONE]` alone is treated as complete without checking length/tool finish
reasons; coding/research substring checks can falsely pass incorrect outputs.
These remain open until fixes and meaningful regressions are inspected.

The implementation now routes native artifact Open to an in-app expanded
dialog instead of relying on WebKit popups. The independent real-browser test
with native transport enabled passes readable content, initial close focus,
Escape, and return to Open. The browser-preview Blob branch was verified
before this split. Actual WebKit Save behavior remains native acceptance work.

Revised diagnostic source now excludes incomplete rows, averages even-sized
medians, requires stop/nonempty/no-tool-call completion and uses restrictive
nonexecuting fixture checks. Final associated regressions and native integration
remain to be audited.

Two additional real-browser negative cases were added and reproduced failures:
`exitCode: 0` plus a failed structured result incorrectly displays a finished
badge, and switching conversations leaves dictation running. They are in
`e2e/review-acceptance.spec.ts`, reported to the implementation task, and keep
R11/R28/R29 open until repaired and retested. The voice test uses a mock speech
service and asserts cancellation plus suppression of later results, not merely
that a label changes.

First coordinated visual batch completed in
`.impeccable/review/review-round-1/`, with consolidated `FINDINGS.md` delivered
to the implementation task. Thirty captures span desktop/narrow light/dark,
conversation, expanded tool/artifact, project documents, generation and
diagnostics. Observations record no page errors, no document-level horizontal
overflow, and no running reduced-motion animations. The narrow Diagnostics
page itself overflows 24 px; other defects include overlapping Latest/composer
controls, nested code frames, light-mode text contrast, old web-card source
validation/title truncation, and verbose composer metadata. These require one
consolidated repair pass and at most one confirmation capture batch.

Native Save now follows a Save-dialog/native-writer path rather than a data
download. The independent browser fixture verifies selected basename/path,
exact literal content/MIME passed to the writer, success feedback, and that
cancelling the dialog calls no writer. These are transport-boundary checks,
not proof of actual OS dialog or disk-write behavior; native acceptance remains
owned by the implementation task.

The dictation conversation-switch regression now passes: the session aborts
and late results do not enter the new draft. Explicit structured-result failure
also now takes priority over process exit zero; the badge is failed, with Exit 0
retained as diagnostic detail and no success badge. Both independently reproduced
defects are repaired in the current browser flow.

The parent inspected native artifact captures under
`.local/qa/review-2026-10-17-native/`. At this audit point,
`native-artifact-saved.png` still shows the GTK Save dialog and the requested
`saved-latency.csv` is absent. This is not accepted as a successful native Save;
the implementation task has been asked to finish the actual dialog workflow
and compare the resulting file bytes. Capture filenames alone are not evidence.

Two additional screenshot-free regressions now guard the first visual batch:
narrow Latest/composer interactive-control collisions, and expanded web-source
safe links/deduplication/readable titles. Their final run is held until the
consolidated style repair is ready, alongside the sole confirmation capture
batch. Current acceptance file has 16 cases.

The native Save workflow is now completed through the actual GTK accessibility
surface. Parent independently checked `saved-latency.csv`: it exists, is 30
bytes, matches `workspace/latency.csv` byte-for-byte, and has SHA-256
`d92c1875dc1454f7a885180afb5edac3f53168b8e52e76e6a87ed4a55a13cab9`.
`native-acceptance.json` and `native-atspi-artifact-{open,saved}.png` retain the
result. The earlier misleading image was renamed to save-attempt1. The previous
missing-file observation is resolved; source/browser/native scope is explicit.

An isolated activity-partition benchmark is replayable with
`node --import ./scripts/register-test-loader.mjs .impeccable/review/activity-benchmark.mjs`.
`activity-performance.json` records implementation hash, CPU/Node metadata,
identical output assignments, five warm-ups and 15 alternating-order paired
trials per size. At 1000 turns/5000 events, previous full scans had median
12.34 ms versus current 0.244 ms; at 2000/10000, 47.85 ms versus 0.361 ms.
This measures only event partitioning, not React paint or inference. The small
100-turn case has noisy/worse p95 and must not support a blanket speedup claim.

The complete browser acceptance run passed 15/16. Its one failure counted the
Latest button as colliding with itself after relocation inside the composer;
that test-only defect was corrected and its focused rerun passed. A final
combined run will follow the final UI confirmation/native adjustments.

The independent combined browser acceptance now passes all 16 cases in one
run (`.local/qa/review-acceptance-confirmation.log`). Actual 200% browser zoom
was separately verified using the Chrome tabs API in an isolated, disposable
Chromium 156 profile. The browser reports zoom factor 2; its CSS viewport changes
1440×1000 → 720×500 and DPR 1 → 2 while document CSS zoom remains 1.
Conversation/composer, palette keyboard focus/Escape and loaded diagnostic
results have no observed overflow/page errors. No screenshots or user-profile
changes were made. The replayable script and report are
`.impeccable/review/browser-zoom-{check.mjs,acceptance.json}`; its context and
temporary profile were removed. API reference:
<https://developer.chrome.com/docs/extensions/reference/api/tabs#method-setZoom>.

The actual native application also persisted an ordinary Quick answer `4`
without reasoning, with capability enabled and measured preparation/first
visible answer/round/total timings. Its in-app diagnostic reports six completed
streams (two primes, four measured), all four measured arithmetic fixtures
correct, spanning tools on/off and requested cold/warm cache. Evidence:
`.local/qa/review-2026-10-17-native/native-runtime-acceptance.json` and
`native-in-app-diagnostic.json`. These synthetic tests do not establish a general
application speedup or live web-search correctness.

The sole final coordinated UI batch passed all eight scenarios with 30 saved
captures. Representative rendered scenes from all repaired surfaces were
inspected. All observation files report no page errors, document/container
overflow, running reduced-motion animations, or text-contrast candidates.
All five accents passed bounded button/accent/ANSI-token contrast checks in
both themes. Detailed confirmation and scope are recorded in
`.impeccable/review/review-round-2/CONFIRMATION.md`. No additional style capture
round is planned.

The broader screenshot-free workflow run initially found a real cancellation
regression: passing onPause hid Stop entirely. Chat now exposes an independent
Stop action alongside optional Pause, including preparation and failed/pending
pause states. Both focused reproductions pass. The final combined run then
passed all 30 browser cases (review acceptance plus existing draft, project,
pause/cancel, trajectory, editor, provider and runtime-control workflows):
`.local/qa/review-browser-final.log`.

The phase-metric persistence/fork regression now asserts preparation, tool,
approval, reasoning and first-visible-answer values survive a reopened database
and fork. Monotonic timer boundaries in chat.rs were inspected. The actual
ordinary native response verifies preparation/first-token/first-visible/total
timings; it did not invoke a tool or wait for approval, so no live forced-approval
measurement is claimed. Missing phases remain null/unavailable.

Actual native project-document acceptance also passes through the GTK/WebKit
accessibility surface. The isolated project enabled guide.md explicitly, found
it, retrieved edited contents, excluded the deleted file, then disabled/deleted
the saved selection; no network was used. Evidence is
`.local/qa/review-2026-10-17-native/native-document-acceptance.json`. Earlier
helper attempts failed to match WebKit accessible field names that included
placeholder text; the corrected prefix matcher completed the actual workflow.
Those harness failures are not represented as product acceptance.

Current-source production build (including TypeScript) passes. The complete
frontend suite passes 529 tests with one skipped; native library tests pass
449 with five ignored. Logs are archived in `.local/qa/review-{build,front,native}-final.log`.
The production build retains the existing advisory about a bundle over 500 KiB;
this is not a build failure or a measured runtime-performance claim.
Strict Clippy also passes after the final native changes. Independent process inspection found no remaining marked review helper/application processes.

## Final disposition

All 31 requirements have implementation and bounded acceptance evidence recorded
above. No unresolved product failure remains from this review. The worktree was
already dirty and was preserved; no release/deployment or blanket staging was
performed. The actual native checks used isolated data/config/workspace and
display sessions, not the user database.

The current-source strict check is `cargo clippy --manifest-path
src-tauri/Cargo.toml --all-targets -- -D warnings`; its successful log is
`.local/qa/review-clippy-final.log`. Native document acceptance was repeated
after the final project-root binding change. No marked review processes remain.

Verification limits remain explicit: live external APIs were not exercised
without configured credentials; subscription protocols remain unavailable;
physical microphone/speech-provider service was not used; browser speech tests
exercise permission/start/result/cancel/switch/error states with a service mock.
Native ordinary-response timings and all phase persistence are verified, while
tool/approval timing code was inspected rather than live forced for measurement.
The diagnostic records the installed model's research-citation failures instead
of treating them as passes. Neither small inference samples nor the isolated
activity-partition benchmark establish a general end-to-end speedup.

User-facing behavior and entry points are documented in README.md; the separate
implementation task's final summary is in REVIEW_REMEDIATION_2026-10-17.md.

Final phase-display coverage added one frontend regression; the final production
build still passes and the final complete frontend suite is **529 passed,
1 skipped**. Final release logs replace the earlier copies under `.local/qa/`.
The implementation report and source/evidence manifest are complete. Independent
SHA-256 checks found no mismatch among the 12 recorded implementation sources.
