# Frontend audit and simplification

This audit started on 2026-10-07 and continued on 2026-10-08. Findings were traced through actual TypeScript callers, React effects, Tauri command registrations, and the Rust session-event query. Graphify output was not used as proof of implementation. Existing uncommitted work was preserved.

## Verified fixes

| Finding | Source evidence | Change and verification |
| --- | --- | --- |
| Activity history required repeated full scans for every tool call | `ActivityTimeline.tsx` used `events.find`, `events.some`, and `messages.find` inside the tool-call loop | Index tool results by run/call, completed runs by run ID, and live messages by ID. A regression with 600 events/200 tools verifies correct results for reused tool IDs and linear event reads. |
| Session pagination rebuilt and sorted all previous events on every page | `useSessionActivity`; `Store::session_events` actually queries unique rows with `seq >= cursor ORDER BY seq ASC` | Append the ordered page and advance to its last sequence. Existing cursor and stale-conversation tests pass. |
| A late runtime poll could replace a stopped runtime with an old ready response | `runtimeStatusPoll.ts` used an interval and unconditional `then(apply)` | Dispose in-flight responses when the phase/callback changes; schedule the next poll after completion. New stale-response and slow-request tests reproduced both defects before the fix. |
| A stream without reasoning still reparsed Markdown on a thinking-clock timer | `MessageBody` started a 100ms timer for every streaming message | Run the clock only for actual reasoning; update once per second. An idle-stream render-count regression failed before and passes after. |
| Switching chats briefly showed the previous chat's checklist | `useConversationTodos` kept unscoped state until the next request completed | Store the conversation ID with the items, so a different chat uses its own fallback immediately. Preserve references when polling returns unchanged rows. Regression tests reproduce both the stale checklist and unnecessary updates. |
| Malformed attachment-envelope JSON could crash the transcript | `MessageBody` assumed every matching envelope had an attachments array containing objects | Check the envelope shape and filter malformed attachment entries; ordinary malformed JSON remains readable. |
| A saved failed response with no generated text disappeared from chat | Compact `ActivityTimeline` rendered the assistant only when text was nonempty | Render error/interrupted states even with empty text, so their saved explanation or stopped state remains visible after reopening. A regression failed before and passes after. |
| Skill dependency inspection invoked a nonexistent command | `api.skillDependencies` called `skills_skill_dependencies`; `lib.rs` registers `skills::skill_dependencies` | Invoke `skill_dependencies`. A test mocks Tauri's actual `invoke` boundary and checks the registered command name. |
| Web search settings performed unrelated execution and cloud polling | `Execution` mounted all three effects regardless of section | Load only the displayed section. Web search performs no interpreter/key reads or Daytona polls; switching sections starts the appropriate effect. |
| Selecting an interpreter claimed testing had started without calling it | `browse` set status `testing` but never called `testInterpreter` | Selecting/editing a path clears obsolete verification; only Test starts verification. Results for an old path are ignored. The verification fallback that reported success without an API was removed. |
| Diagnostics exposed an action that performed no diagnostic operation | Retest tool calling only updated a local notice string | Remove its button, state, and extra wrapper. The real runtime-log refresh and error recovery remain tested. |
| Redundant declarations and activity fields | Identical `Hit` interfaces; timeline fields `filename`, `toolName`, and `rawEvent` were not consumed | Remove duplicate declarations and unused fields; share duplicated tool classification. |
| First-run text did not explain how to get started | Chat displayed a slogan even when no model was loaded | Show Start a local conversation with the next model setup step, or an actionable prompt using the selected project. Keep the existing suggestion actions. |

## Measurements

- The 600-event timeline regression performed **124,000 event-type reads before** and **4,800 after**: a **96.1% reduction** in those reads, with result association preserved. This is an algorithmic work measurement, not a measured increase in GPU inference speed or browser FPS.
- Reasoning-clock updates decrease from 10/sec to 1/sec. Streams without reasoning now cause **zero timer-triggered Markdown renders**; the test reproduced an extra render before the fix.
- Web search settings avoid two unrelated initial IPC calls and the Daytona request on mount plus 12 requests/minute while that page is open.
- Production source in the frontend agent's eight edited files is **23 lines shorter, 22 nonblank lines shorter**, compared with the entry snapshot. Tests add coverage and are counted separately. Previously deleted user files are not credited to this work.
- Focused validation covers **79 tests across 13 files**: the 13-file run passed 78 tests, followed by the new saved-failure regression and a 46-test Chat/timeline run. `npx tsc --noEmit` passed. A focused oxlint run on edited production and test files passed with warnings denied. Repository-wide checks are reported by the main audit.

## Feature coverage and product decisions

| Feature | Actual implementation and checks | Decision |
| --- | --- | --- |
| Local chat and model choice | `Chat`, `Models`, `ModelLibrary`, `RuntimeForm`; tested selection, load-state controls, final response metrics, copy, retry, drafts, and busy handling | Keep: core product flow. |
| Agent work, tools, plans, and context | `ActivityTimeline`, `CommandRunCard`, `WorkSummary`, `PlanChecklist`, `ContextControl`, `ToolControls`; caller and test inspection | Keep: preserve work provenance and permissions while keeping the final answer primary. |
| Downloads and recovery | Shared `downloadJobs` store, `JobProgress`, model/runtime download panels; job state, cancellation, resumption, stale status, and verification tests | Keep: these are recovery controls, not redundant features. |
| Workspace inspector | `WorkspacePanel`, browser, terminal, and file panels; actual imports/callers and existing test paths inspected | Keep: terminal/browser/files/review directly support project work. Startup loading is handled by the main audit. |
| Browser address resolution and lifecycle | `BrowserPanel`; HTTP(S)/host/search classification, show/navigation sequencing, unmount cleanup tests | Keep; native overlay behavior still needs desktop acceptance testing. |
| Model telemetry and diagnostics | Hardware readings, memory estimates, actual runtime-log API; source inspection and diagnostics tests | Keep real telemetry; remove the fake retest action. Estimates remain labelled. |
| Connectors and skills | Catalogs, encrypted local connector configuration, activation, file inspection, and dependency command | Keep: optional capabilities remain configured through settings. Fix the broken dependency command. |
| Execution and historical cloud recovery | Local interpreter tests and explicit unsandboxed execution; Daytona cleanup journal and credentials | Keep local execution. Retain historical cleanup because removing it could strand external resources; do not offer unavailable cloud execution as a new feature. |
| Web search | Provider settings and connection-status tests; actual native API names inspected | Keep: optional retrieval supports the agent. Avoid unrelated polling on this page. |
| Session history and developer trajectory | `Trajectory`, session APIs, pagination and fork paths | Keep for provenance and troubleshooting; broad navigation simplification is handled by the main audit. |

Frontend-unused API declarations such as capabilities/config inspection, pause/resume/restart, replay, and goal retrieval are not enough evidence to delete their native features. Some are exercised by smoke scripts or used as backend harness capabilities. Deleting native behavior based only on a lack of a React button would be a false audit conclusion.

## Remaining limits and next steps

1. Acceptance-test the packaged Tauri app with the user's model and GPU. Browser tests and jsdom cannot verify actual inference throughput, GPU memory allocation, native child-webview overlays, Windows PTY behavior, or download integrity against live services. The comprehensive audit separately ran a brief direct managed-MiniCPM fixture and native PowerShell PTY tests; those narrower observations do not certify the desktop UI flow or sustained throughput.
2. Test OAuth sign-in and remote connector operations with authorized test accounts. Frontend fixtures do not prove an external account flow works today.
3. Keep a short real usage performance profile: opening a large project, a long streaming transcript, switching chats, and a large download. Use its measured bottlenecks to choose the next optimization; do not add speculative caches or abstractions.
4. Production rendering is bounded by individual turn contents even though old turns are memoized. Virtualization should be considered only if real long-session profiles show DOM/render cost after these fixes.
5. Some async settings operations, including skill file preview selection, can still resolve after navigation. Current effect reads generally discard stale responses; further work should target reproduced user-visible races rather than adding generic request infrastructure everywhere.
6. Full-suite Vite browser startup can exceed a 5-second assertion timeout under concurrent compilation. The main audit measured successful module responses rather than treating a slow load as a missing Models feature; concurrency and an explicit startup wait belong in the harness.

No claim is made that every live integration or every possible combination of tools has been exhaustively proven. The documented fixes have focused regression evidence, and the remaining acceptance work is explicit.

## Independent review of web and worker changes

Reviewed the other agent's actual search/cache entry points, provider cache keys, document age checks and redirect identity, result deduplication, complete-pool reranking, chunking, domain-health window reset, requirement matching, time-zone resolution, and pinned transport implementation. Existing tests were checked for the intended invariants rather than treated as proof solely because they passed.

The review found one additional transport defect: an unavailable `robots.txt` HTTP response was cached as permanently permissive. The web agent reproduced it with a failing 503-to-recovered-denial regression, evicted transient HTTP outcomes, and kept the intended soft handling of the immediate failure. Independent `node --test scripts/web-transport.test.mjs` validation then passed **9 tests**, including HTTP diagnostics, private redirect blocking, robots denial, Retry-After cooldowns, immediate cold-host starts, and failure recovery.

No further blocking finding was identified in the reviewed change paths. Requirement coverage remains a lexical heuristic, and pinned transport fixtures do not establish current Google/SearXNG/Wayback integration availability. These limits are retained in the web audit.
