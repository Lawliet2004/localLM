# LocalLM source audit and improvement plan — 7–8 October 2026

## Scope and evidence

Audit the current working tree, including its existing uncommitted changes. Graphify output is not evidence of behavior: conclusions require source call paths, runnable checks, or runtime observations. A temporary copy of the starting source is retained outside the repository for comparisons. No user changes are reset or committed.

The product remains a local Windows agent over a project folder. Chat, model/runtime management, project files, terminal/browser/review, research, tools, skills, connectors, permissions, history and export are supported surfaces. GPU throughput and external service availability need separate live verification; mocked browser tests cannot establish those.

## Baseline

- 353 source/test/script files; 71,064 lines (including tests and blank lines).
- Initial production JavaScript: 1,033.44 kB, 301.62 kB gzip. CSS: 110.25 kB, 19.81 kB gzip.
- Typecheck and lint pass. Vitest: 491 pass, one skipped; 44.88 s on this machine.
- Script tests: 40 pass. Edge browser tests: 11 pass, one failure in model navigation, to investigate against current source.

## Ordered plan

| Priority | Source-confirmed problem | Action | Acceptance evidence |
| --- | --- | --- | --- |
| P1 | Terminal and file-highlighting libraries load during startup although the inspector is closed | Load inspector content when requested | Smaller startup bundle; all inspector tab/keyboard tests pass |
| P1 | A generation failure restores already-persisted input | Reconcile acceptance before restoring drafts | Persisted and rejected-send regression tests |
| P1 | Workspace traversal holds the shared database mutex | Snapshot the root under the lock, traverse on a blocking worker | Rust tests and native command compilation |
| P1 | Research context reads entire chat history twice | Use bounded SQL projections | Long-history equivalence test and benchmark |
| P1 | Web transport delays cold requests and mishandles cooldown retries | Honor retry limits and cooldown while preserving host spacing | Deterministic transport failure/rate-limit tests |
| P1 | Configured reranking skips scoring all passages | Restore actual relevance scoring | Query relevance regression |
| P2 | Timeline building repeatedly scans events/messages | Index lookups once; keep event order | Timeline regression/performance check |
| P2 | Runtime polling overlaps and accepts stale responses | Serialize polls and ignore disposed requests | Deferred-promise regressions |
| P2 | Sidebar repeats settings links and project-delete actions | Keep one Settings entry and contextual project actions | Sidebar, keyboard and browser verification |
| P2 | Browser overlay tracking rescans during transcript mutations | Profile native menu/dialog visibility before batching scans | Deferred: lifecycle/geometry checks pass, but safe overlay timing needs a native WebView2 observation |
| P2 | Expired domain learning keeps an expired window start | Restart the rolling window correctly | Fake-clock learning regression |
| P2 | Worker builds leave temporary output behind | Clean temporary output after both success and failure | Script checks |
| P3 | Potentially redundant legacy modules and advanced options | Verify callers and persistence compatibility before deletion | Feature map and explicit keep/defer decisions |

## Simplification policy (ponytail: full)

Delete proven dead scaffolding and duplicate presentation, use existing APIs and native controls, and avoid speculative dependencies. Measure nonblank production code separately from tests; removing blank lines is not a performance improvement. Do not remove validation, accessibility, approvals, recovery, or data compatibility to achieve a line-count target. The one necessary dependency addition is pinned `chrono-tz`, replacing an incorrect hand-written seasonal offset table with IANA rules.

## Verification and feature decisions

## Implemented changes

The work follows the existing `PRODUCT.md` and Graphite `DESIGN.md`, rather than inventing a different product. Changes are compared with the saved entry working tree, which already contained extensive user edits.

| Area | Result |
| --- | --- |
| Startup | Terminal and file highlighting load only when requested. Browser event registration remains available at startup. Lazy tab switching, closing, resize and keyboard behavior are tested. |
| Chat reliability | Accepted persisted prompts stay cleared after generation failure; rejected prompts and failed reconciliation retain their drafts. Missing-project sends retain input. Progress-only events do not create phantom assistant messages. Empty saved failures/stops remain visible after reopening. |
| Projects and navigation | One sidebar Settings entry replaces the duplicate brand menu. Project deletion stays in its contextual menu with confirmation. Unique project IDs prevent identically named folders from silently replacing each other's paths; failed saves do not start a chat. |
| Transcript work | Indexed timeline lookups, append-only ordered event pagination, fewer reasoning-clock updates, and chat-scoped checklists avoid repeated work and stale state. Malformed attachment envelopes no longer crash rendering. |
| Setup and settings | First-run guidance explains model setup. Fake diagnostic actions and fake interpreter verification states are removed. Web search settings no longer fetch execution credentials or poll cloud state. Skill dependency IPC calls the command actually registered by Rust. |
| Database and workspace | Research context uses bounded projections instead of loading full history twice. Startup repair avoids unused payloads. Workspace traversal releases the database lock and runs on a blocking worker; project membership uses a set. |
| Native tools | Git output is bounded while pipes are drained and independent reads run concurrently. Terminal output preserves split UTF-8 characters and capped replay. Native browser navigation/popups enforce the supported URL schemes. Clock tools reject malformed Unicode offsets and use date-correct IANA offsets. |
| Research and web | Cold hosts start immediately, retry/cooldown limits are respected, transient robots failures recover, search/document caches are used, complete fetched documents remain available to open/find, reranking actually scores, requirement coverage respects subjects, chunk overlap and domain-learning windows are corrected. |
| Maintenance | Removed confirmed unused web modules/helpers, duplicated declarations, dead installer fixture scaffolding and worker build leftovers. Patched `source-map-js` to 1.2.2. Web tests run in Node while DOM tests retain jsdom. |

## Measured results

| Measurement | Entry | Final | Interpretation |
| --- | ---: | ---: | --- |
| Initial production JavaScript | 1,033.44 kB | 539.77 kB | 47.8% smaller startup JS; terminal/files download when opened |
| Initial JavaScript gzip | 301.62 kB | 163.84 kB | 45.7% smaller compressed startup JS |
| Initial CSS / gzip | 110.25 / 19.81 kB | 105.69 / 18.94 kB | Terminal CSS is also deferred (3.93 / 1.01 kB) |
| Research context, 1,003-message fixture | 20.8803 ms | 0.4844 ms | About 43× faster SQLite preparation, with equivalent output |
| Title deduplication, 150 synthetic comparisons | 9,990.7 ms | 317.0 ms | About 31.5× faster helper; rolling-row memory instead of a full matrix |
| Timeline, 600 events / 200 tool calls | 124,000 event-type reads | 4,800 reads | 96.1% less measured lookup work |
| Vitest wall time, observed full run | 44.88 s | 24.69 s | More tests, fewer unnecessary jsdom environments; machine load affects timings |

These are local bundle sizes, algorithmic measurements and fixtures. They do not establish an increase in GPU tokens/sec or a measured desktop cold-start time. The build still reports its 500 kB chunk warning for the remaining 539.77 kB core; that warning was not hidden by raising its threshold.

Across 63 changed source/test/script files, application code is **215 lines shorter (201 nonblank)**. Regression tests add **486 lines (452 nonblank)**, so combined source and tests increase by 271 lines. Rust inline test modules are counted as tests, and generated bundles, dependencies, config and documentation are excluded from this comparison. Earlier user deletions are not credited to the audit. The source reduction is intentionally modest: bounds, recovery and correctness are retained.

## Final verification

| Check | Result |
| --- | --- |
| `npm run typecheck` | Pass: app and e2e TypeScript |
| `npm run lint` | Pass: warnings denied |
| `npm test` | 518 passed, one optional live-model test skipped; 85 files passed, one skipped |
| `npm run test:scripts` | 44 passed, including the regenerated bundled worker, transport and mediated rendering |
| `npm run build` | Pass; worker rebuilt from final source, Vite production assets generated |
| `npx playwright test` | 13 passed in Edge; all 15 settings sections, catalogs, theme, responsive widths, inspector/shortcuts, permission persistence, downloads and outage recovery |
| `cargo test` in `src-tauri` | 449 passed, two ignored; binary and doc-test targets also compile/pass |
| Explicit ignored native/cache tests | Both passed separately: managed runtime flags and two cached pinned CUDA archives |
| Optional cached MiniCPM fixture | Passed separately before and after its test-only evidence repair; two fixture passes per run |
| `cargo clippy --all-targets -- -D warnings` | Pass |
| `npm audit --json` | Zero reported npm vulnerabilities |
| `git diff --check` | Pass; an extra blank EOF in the entry-baseline `RuntimeForm.test.tsx` was trimmed without changing its tests |

Browser runs originally hit a lazy settings load timeout. Successful network responses showed a cold Vite transformation waterfall under concurrent compilation. The harness now uses two Edge workers, a ten-second expectation timeout and documented Vite server warmup for App/Models; the final complete run passes. This development-server adjustment does not eagerly execute lazy production components. Screenshots were inspected in dark/light chat and 768px Models/Runtime; tests also check overflow at 320–1440px and transcript/inspector layouts down to 390px.

The optional cached MiniCPM fixture also passed separately: one test containing two direct-server fixture passes in 65.06 seconds, including 44.56 seconds of Rust compilation. It exercised completion from supplied evidence and the shipped extractor's compressed PDF text, a forced model-generated `run_code` calculation handed into real workspace execution and saved/read back as `21.59`, and HTTP stream abort followed by a successful `PONG` response. This briefly loaded the cached 2.07 GB GGUF through the managed runtime; it is not a sustained throughput benchmark or a full native desktop chat run.

Independent fixture review found misleading labels and an external-request recorder bypassed by direct `originalFetch` calls. Correcting the test-only labels, recorder and hidden Windows process launch was followed by a successful rerun in **15.60 seconds**. The recorder actually observed 26 JavaScript fetches, including health and at least 18 completions, with zero external JavaScript requests. Native child-process traffic remains outside its observation. The fixture's manually seeded checkpoint/ledger paths do not constitute a real desktop process restart, long-context compaction or proof of offline/network isolation. The detailed web report records the precise scope; separate Rust tests exercise actual local-only dispatch and compaction contracts.

## Feature decisions

“Tested” below means the stated source/tests/runtime observations, not exhaustive proof of every tool combination or live service.

| Feature group | Evidence and status | Decision |
| --- | --- | --- |
| Local chat, drafts, streaming, attachments, model choice | App/Chat/streaming/attachment regression tests and Rust dispatch paths; direct cached-model fixture reported separately | Keep: core flow. Full desktop model/GPU acceptance remains required. |
| Project files, search, editing, Git, inspector | Real file/Git fixtures, project binding/path bounds, React tests and Edge inspector/layout checks | Keep; load expensive inspector content on demand. |
| Terminal and code execution | Actual local execution/descendant cleanup and PowerShell PTY shutdown tests; Unicode/replay regressions | Keep; execution remains explicitly unsandboxed as documented. |
| Browser and browser harness | Native command/driver paths exist; mode/action budgets, URL/geometry/lifecycle tests pass | Keep optional controls. A real native child-WebView2/agent interaction is still unverified. |
| Research, search, page open/find, PDF text | Web tests, complete-document and cache regressions, bundled-worker tests, actual pinned public HTTPS fetch | Keep. Search/weather/archive services and OCR are not certified by fixtures. |
| Models, runtime, downloads, hardware | Install/recovery/range/hash tests, cached real archive validation, all Models settings screens in Edge | Keep reliable setup/recovery; sustained GGUF loading, VRAM and throughput need native acceptance. |
| Tools, plans, permissions, approvals | Registered command paths, scope/single-use validation, checkpoints, run history and UI tests | Keep; these controls prevent lost state and unintended writes. |
| MCP connectors, skills and credentials | Actual local stdio protocol processes, encrypted-store/path/integrity tests and catalog/UI tests | Keep optional extensibility. Remote OAuth/account operations remain unverified. |
| History, archive, fork, export, memory | Atomic persistence/replay, recovery/fork/export fixtures and frontend paths | Keep. Very large histories remain a measured follow-up candidate. |
| Diagnostics and trajectory | Actual logs/session events exist; frontend viewer tests pass | Keep real inspection, remove actions that only pretend to test. |
| Legacy remote/subscription/cloud cleanup | Source callers, saved formats and owned-resource journal depend on them; local-only/new-cloud gates are tested | Defer deletion until a tested compatibility migration. Do not add new cloud execution. |
| Duplicate menus, dead query/vertical schemas, unused ranking/extraction helpers | Repository caller searches and typecheck confirm no required consumers | Removed. |

## Next improvement plan

Prioritize measured reliability and task completion over more providers, dashboards or orchestration. Keep the application focused on a local model doing useful work in a project folder.

| Priority | Next work | Scope and acceptance |
| --- | --- | --- |
| P1 | Native release acceptance | Launch the packaged app with a selected GGUF/GPU; measure cold launch, first token, tokens/sec and memory. Exercise stop/retry, reopen, terminal Unicode, native browser overlay and a real model-driven file task. Use disposable projects and record actual outcomes. |
| P1 | Make research settings match behavior | Reconcile normal/deep advertised search/page budgets with the staged planner's own limits (including its four-query cap). Tests must trace each visible control to the dispatched budget rather than only persistence. |
| P1 | Strengthen live integration evidence | Test current SearXNG/search/weather/archive services and OAuth/MCP with authorized test accounts. Replace seeded resume/compaction claims with real process lifecycle fixtures and meaningful network observations. |
| P2 | Large-history responsiveness | Profile thousands of chats and long transcripts first. Add SQL/event windows and then targeted rendering virtualization only where measurements show a bottleneck; preserve ordering, search, drafts and recovery. |
| P2 | Repeated tool setup cost | Measure secondary `Store::open` schema work and Node worker startup/integrity verification. Reuse only if worthwhile, preserving secondary-connection non-recovery and worker isolation. |
| P2 | Remaining settings races | Reproduce late skill/file-preview results across navigation before changing them. Use local request identities rather than introducing a generic request framework. |
| P2 | Browser overlay scan cost | Measure body-wide mutation/visibility scans during streaming with the native browser open. Batch or narrow only after menu/dialog hiding remains immediate and native geometry is verified. |
| P3 | Legacy code migration | Inventory persisted provider/audit/cloud ownership formats, migrate them with compatibility fixtures, then delete adapters no longer reachable. Preserve cleanup of owned resources. |
| P3 | Further core-bundle reduction | Profile Markdown/icon parsing and startup before splitting more components. Preserve immediate first-response rendering; do not trade a smaller displayed bundle number for a slower useful first interaction. |

## Detailed evidence

- [Backend audit](BACKEND-AUDIT-2026-10-07.md): exhaustive 70-file Rust feature map, command/recovery review, real local fixtures and benchmark.
- [Frontend audit](FRONTEND-AUDIT-2026-10-07.md): individual UI/state fixes, focused regressions, source decisions and independent web review.
- [Web audit](WEB-AUDIT-2026-10-07.md): retrieval feature map, cache/security behavior, benchmark reproduction and live-provider limits.

No Graphify output was accepted as proof. Agents inspected actual source, independently reviewed each other's change paths, reproduced defects before repairs where practical, and ran the final checks against the shared resulting working tree. No commit, deployment, remote message or destructive migration was performed.
