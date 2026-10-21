# Full-stack implementation report — 2026-10-16

The application now integrates native Bash/Python execution, dedicated weather,
validated tools, persisted response metrics, Git authoring and complete Settings
destinations with the existing runtime, research, editor and automation systems.
This extends the already modified application. No user changes were discarded,
no repository commit was created, and conversations, credentials and settings
were not reset.

## Implementation and principal modules

| Request | Delivered behavior and main files |
| --- | --- |
| Audit | Inspected existing architecture, prior audits, settings and working tree before implementation; ran fresh full baseline suites. Scoped completion audit: [IMPLEMENTATION_CHECKLIST.md](IMPLEMENTATION_CHECKLIST.md). |
| Web retrieval | One SearXNG planning/search/BM25/evidence pipeline with Fast/Balanced/Deep budgets, real JSON health checks, instance failover and graceful no-model/provider errors. Added JSON/CSV/TSV extraction without HTML rendering. Persistent caches prune by TTL, count and serialized bytes; clear-cache retains saved research/citations. Existing pinned DNS/redirect transport, bounded fetches, PDF text extraction, cancellation and compact cited evidence remain shared. Files: `src/web/`, `scripts/web-worker.mjs`, `scripts/web-storage.mjs`, `scripts/web-transport.mjs`, native `web_search.rs`. |
| Native execution | Configurable Bash on Linux and platform-appropriate Windows shell; independent interpreter availability, policy, cwd, environment, timeout and optional memory settings. Python snippets/files support project imports, existing venvs, arguments, stdin, tracebacks and generated files. File approvals snapshot source and verify its hash again before execution. Bounded structured output and declared artifact metadata; text and PNG/JPEG/WebP previews bind to the workspace and content hash. Files: `execution.rs`, `workspace.rs`, `artifacts.rs`, `NativeExecutionSettings.tsx`, `ExecutionArtifacts.tsx`. |
| Terminals | Persistent user xterm PTYs retain ANSI, input, interrupt, resize and scrollback. Native output uses bounded buffering, one acknowledged chunk in flight and an overload notice. Closing kills the foreground group and shell. The model's persistent command session now actually uses configured Bash/PowerShell, keeps cwd/variables, reports both streams/exit status and invalidates cancelled/timed-out sessions. Files: `terminal_ui.rs`, `sandbox.rs`, `TerminalPanel.tsx`. |
| Weather | Validated `get_weather` uses Open-Meteo directly, independently of SearXNG/model inference. Current conditions, up to 48 future hourly entries and 1–7 daily forecasts include place, timezone, units, fetched time and source. Explicit place or saved preference, C/F, exact-place preference, ambiguity errors, 10-minute persistent cache, offline/enable gating and result widget. Files: `src/web/verticals/weather/`, native `weather.rs`, worker, `WeatherSettings.tsx`, `WeatherResult.tsx`, `ToolResultExtras.tsx`. |
| Tools/permissions | Common JSON Schema validation for built-in/MCP schemas: offline references, bounded documents and cached validators. AREX types normalize before validation. Effective execution arguments appear for approval; stricter per-tool approval overrides Full access. Foreground/background/child/PTC honor policy/cancellation. Dropped approval futures clear pending records. Invalid inputs persist failures. Unavailable tools are omitted; `run_code` reuses the native executor without duplicate definitions. Files: `tool_validation.rs`, `agent_run.rs`, `approval.rs`, `arex.rs`, `harness.rs`, `subagents.rs`, `presets.rs`. |
| Runtime | Existing authenticated loopback lifecycle, backend selection, readiness, cache telemetry, prompt budgeting, streaming and recovery retained. Validation now parses GGUF metadata before replacing a loaded runtime. Diagnostics performs a real cancellable structured tool-call probe, with results tied to runtime/model/template/parser fingerprint. Completed Markdown responses are memoized following a reproduced render regression. Files: `runtime.rs`, `runtime_config.rs`, `inference.rs`, `tool_compat.rs`, `chat.rs`, `Chat.tsx`. |
| Response metrics | Native counts/rates/durations, model, cache, per-round completion and monotonic first token. Additive `message_inference` storage survives restart/forks. GEN/PROMPT/TTFS and expandable details show unavailable data honestly. Removed an invented reasoning-duration label. Files: `inference.rs`, `chat.rs`, `store.rs`, `lib/responseMetrics.ts`, `ResponseMetrics.tsx`. |
| Browser/computer | Independently selectable, disabled-by-default modules, history, approvals, isolated per-conversation browser sessions, screenshots and cancellation cleanup retained. Real browser and isolated desktop acceptance rerun. Browser navigation/inspection/forms/scroll/screenshots/tabs stay separate from native pointer/keyboard/capture. Files: `automation.rs`, `computer_use.rs`, `AutomationSettings.tsx`; setup: [AUTOMATION.md](AUTOMATION.md). |
| Git | Native literal-path subprocesses, bounded output/deadlines and no interactive credential prompts. File status/diffs, stage/unstage, commit confirmation bound to the reviewed staged tree, branch list/create/safe switch and 50-entry history. Commit rejects a workspace nested inside a larger repository to keep review scope honest. Files: `workspace_ui.rs`, `ReviewPanel.tsx`, `GitSettings.tsx`, `WorkspacePanel.tsx`. |
| Settings/UX | Models & runtime, Inference, Tools & permissions, Web Search, Bash, Python, Weather, Browser Use, Computer Use, Git, Security, Appearance and Diagnostics use real state. General/Execution/Connectors/Skills retained. Nonzero interpreter tests cannot report success; failed settings reads cannot overwrite saved values with defaults. Existing tokens/IBM Plex/themes, responsive navigation, drafts, validation and focus retained. Files: `App.tsx`, `SettingsNavigation.tsx`, settings components, `desktop.css`, `tokens.css`. |
| Linux credentials | Fixed production keyring configuration that used a nonpersistent mock backend on Linux. Now uses Linux Secret Service, Windows Credential Manager or macOS Keychain. Provider bootstrap checks credential presence without decrypting every key; locked/broken stores fail explicitly on use. Files: `Cargo.toml`, `vault.rs`, `providers.rs`. |

Rust filenames are under `src-tauri/src/`; React components under
`src/components/`. Existing MCP, skills, storage and editor behavior was retained.

## Executed verification

Host: Linux X11, Node 22.22.1, installed Python/Bash, RTX 2050 with 4 GiB VRAM.
Native desktop tests use isolated Xvfb and temporary app data.

| Command/check | Actual result |
| --- | --- |
| `npm test` | **509 passed, 1 skipped**, 81 files. Skip: opt-in live MiniCPM. Baseline: 490 passes/1 skip. |
| `npm run typecheck` | Passed. |
| `npm run build` | Passed, including bundled worker. Existing minified chunk-size advisories remain. |
| `npx playwright test --workers=4` | **34 passed**: settings, drafts, keyboard/themes/resizing, editor, browser layout/motion, automation gating and regressions. |
| `cargo test --manifest-path src-tauri/Cargo.toml` | **432 passed, 5 ignored**, no failures/filtered tests. Baseline: 411 passes/5 ignored. |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | Passed. |
| `npm run test:web-transport` | **19 passed**, pinned transport/restrictive renderer, hostile inputs and failures. |
| `node --import ./scripts/register-test-loader.mjs --test scripts/searxng-worker.test.mjs scripts/weather-worker.test.mjs scripts/web-storage.test.mjs` | **9 passed**, bundled protocol/failover, weather errors/cache across workers and cache migration/bounds/clear. |
| `npm run test:grounding` | Passed research/grounding smoke. Pipeline accounting/citations, not a live model performance benchmark. |
| Opt-in `automation::tests::real_browser_form_tabs_screenshots_isolation_and_cleanup` | **1 passed**, actual Rust → pinned Playwright MCP → Chrome. |
| `node scripts/test-desktop-automation.mjs` | **1 passed**, native X11 pointer/typing/keys/scroll/capture on isolated fixture. |
| `npm run test:browser-layout` | Passed actual Tauri/WebKitGTK navigation/input/bounds/hide/restore/tabs/resize/close. |
| `node scripts/native-linux-smoke.mjs` | Passed native app launch, home capture and Ctrl+, Settings interaction on separate X11/D-Bus/app-data. |
| Native keyring smoke | Temporary Secret Service entry persisted across independent entries **and a second process**, then deleted; no real app key replaced. |
| Native GGUF smoke | Parsed installed 3,219,532,192-byte Gemma 4 file: `gemma4`, 131,072 model context, 42 blocks. |
| Live Open-Meteo | Tokyo, Japan resolved to Asia/Tokyo; Fahrenheit preference, real current/hourly/daily data and fetched timestamp. |
| Installed llama.cpp stream/tool probe | CUDA/CPU streams, cache observations, valid native structured call and owned-process shutdown passed. Probe executes no external action. |
| Design review | Detector returned `[]`; native home/Settings and desktop/narrow Python/Web Search in both themes visually inspected. |
| `git diff --check` | Passed. |

Five ignored Rust tests: real browser, isolated desktop, live Hugging Face,
Windows CUDA flags and Windows archive extraction. The first two passed when
explicitly run; the other three were not run on Linux. Browser API fixtures do
not establish native behavior; the separate native checks supply that evidence.

Raw logs/captures are retained locally in
`.local/qa/full-implementation-2026-10-16/` (ignored development evidence).
Reproducible helpers: `scripts/benchmark-local-runtime.py`,
`scripts/native-linux-smoke.mjs`, and opt-in `src-tauri/examples/`.
The native smoke requires `npm run dev`, Xvfb, `dbus-run-session`, `xwininfo`;
`LOCALLM_XVFB_BIN` can select another binary.

## Measurements and boundaries

Installed `llama-b11669-cuda12.8-linux-x64` ran
`gemma-4-E4B-it-qat-UD-Q2_K_XL.gguf` on identical 662-token requests twice.
Both completed with 36 generated tokens and process shutdown. These are host
observations, not statistical benchmarks or proof that edits sped up llama.cpp.

| Measurement | CUDA | CPU fallback |
| --- | --- | --- |
| Configuration | Saved 32,768 context, auto GPU layers, 6 threads, batch 512/micro 128, Flash Attention, q8 K/V, one slot, projector | Explicit GPU layers 0, 4,096 context, no projector; other relevant settings retained |
| Process cold-start to readiness | 14.807 s | 8.307 s |
| Uncached prompt observed first token | 2,146.44 ms | 4,282.56 ms |
| Identical warm prompt first token | 64.62 ms | 91.93 ms |
| Native cached prompt tokens | 0 → 661 of 662 | 0 → 661 of 662 |
| Uncached prompt processing | 315.55 tok/s | 156.66 tok/s |
| Native generation, first / warm | 18.61 / 19.02 tok/s | 10.35 / 12.18 tok/s |
| Process RSS, loaded / after inference | 2,117,376 / 2,329,496 KiB | 3,321,768 / 3,606,096 KiB |
| Device-wide GPU memory, loaded / after inference | 2,766 / 2,832 MiB of 4,096 MiB | Not attributed to the CPU process; other desktop activity also uses GPU |

“Cold” means fresh process/empty prompt cache, not flushed OS file caches.
CPU/CUDA configurations differ and are not a controlled backend comparison.
GPU snapshots include the desktop. First token is monotonic HTTP-client
observation, not UI paint latency. No inference speedup caused by this patch
is claimed without a controlled before/after experiment.

An earlier 13:53 host smoke (`.local/linux-runtime-smoke.json`) recorded a
16.03-second load and 18.82 tok/s generation on the same runtime/model/settings.
That used a 16-token prompt and two-token `READY` answer, without warm-cache,
streamed-first-token or RSS records. The later 14.807-second load is an
observation, not an attributed improvement: prompts, system load and file-cache
state were not controlled. Missing baseline measurements are not reconstructed.

A controlled UI regression reproduced and removed unnecessary work: five
streaming updates with 16 completed responses caused **80 actual ReactMarkdown
renders before memoization and zero afterward**. The test wraps the real
component; this measures render count, not elapsed milliseconds. Weather tests
verify two initial upstream requests and zero in a new worker using fresh cache,
preserving the original fetched timestamp.

GEN/PROMPT prefer llama.cpp's native rates. Its generation rate can exclude the
initial decode token, so dividing every token count by duration would misstate
it. Multi-round rates are duration-weighted. TTFS runs from first model HTTP
request dispatch to first native-observed content, reasoning or tool token,
including HTTP/stream handling before UI render. Total response duration starts
at native send invocation, includes preparation/tools/approval, and ends before
final persistence/UI delivery. Missing/incomplete timings remain unavailable.

## Start and enable

1. Install Node 22.13+ (24 recommended), Rust and platform Tauri prerequisites.
   Linux credentials also need Secret Service/D-Bus and `libdbus-1-dev`;
   capture/input development dependencies are in [AUTOMATION.md](AUTOMATION.md).
   Windows needs MSVC/C++ Build Tools/WebView2. Run `npm ci`, then
   **`npm run tauri dev`**. `npm run dev` alone is browser preview.
2. **Models & runtime:** install/select runtime and GGUF, save, load. Reload for
   load-time Inference changes; saved/loaded state is distinguished. GPU layers
   `0` selects CPU fallback. Diagnostics → **Retest tool calling** makes a real
   local request and may replace the slot's warm prompt cache.
3. Choose a workspace. **Bash/Python:** detect/browse/test executable, enable,
   configure cwd/timeout/env/optional venv or memory limit, save. Select tools
   in the conversation. Ask shows code/command/effective cwd; “always ask” can
   tighten Full access. Workspace Terminal is an independent manual PTY.
4. **Web Search:** configure self-hosted SearXNG with JSON, Check connection,
   save, enable, choose Fast/Balanced/Deep and select a research tool/preset.
   **Clear retrieval cache** retains saved citations. Service setup:
   [WEB_SEARCH_PHASE1.md](WEB_SEARCH_PHASE1.md).
5. **Weather:** enable and save optional location/C/F; select `get_weather`.
   Supply location if no preference exists. Requires Node/network, no API key,
   SearXNG or model inference. Ambiguous places return choices; device location
   is never inferred. Result attribution includes source/licensing.
6. **Browser Use:** set up pinned engine, choose browser, enable/save. Enable
   **Computer Use** separately and select each in Tools. Computer screenshots
   need a vision model; Linux input needs X11. Both default disabled. See
   [AUTOMATION.md](AUTOMATION.md).
7. Open a Git workspace → **Review**, or **Git → Open Git review**. Stage files,
   enter message, confirm reviewed commit; branch/history controls are nearby.
   Metrics appear under completed responses; expand for details. Security
   exposes offline mode and permission guidance.

## Limits and remaining release verification

- Native commands use normal account files/network. Workspace checks are not
  OS isolation. Unix memory limits bound each process's address space, not an
  aggregate tree; unavailable on Windows. Code calls never install packages
  silently; explicit browser setup is a separate action.
- Previews cover text/PNG/JPEG/WebP up to 2 MiB. HTML is literal text. Other or
  larger artifacts remain workspace files and return explicit preview errors.
- PDFs need extractable text; no OCR or protected-PDF decryption. Optional
  rendering requires compatible Edge/Playwright and fails closed if absent.
- Research tests use real transport plus deterministic upstream fixtures. A
  deployed self-hosted SearXNG instance with every external engine was not
  validated here; Check connection diagnoses the configured service. Weather
  was tested live; Open-Meteo licensing/service terms apply.
- Desktop input was verified on Linux X11. Wayland input is explicitly disabled;
  capture depends on compositor/portal. Desktop actions address the primary
  monitor/focused app; no dedicated window-selection API. Windows/macOS native
  acceptance, installers/signing and a model-driven desktop task are untested.
- Git stages whole files, with no hunk editor or dedicated remote UI. Optional
  remotes can use the approved shell/configured Git credentials. No user commits,
  remotes or force/history-changing operations were performed during QA.
- Authenticated providers/MCP, Daytona, each skill's external workflow, full
  managed-model transfers and crash-recovery/install acceptance need separate
  account/platform tests. Fixture success is not proof those accounts work.
- Secret Service must be available/unlocked for secrets. Previously lost
  mock-backed keys cannot be reconstructed; existing encrypted files remain
  preserved and decryption errors explicit.
- Older responses without metrics show unavailable values. Bundle-size
  advisories remain. Universal model/platform compatibility and zero bugs are
  not claimed.
