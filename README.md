# LocalLM

A local AI desktop workspace built with Rust, TypeScript, React and Tauri 2. Compatible GGUF models run through a managed llama.cpp process, with GPU offloading, chat, MCP connectors, skills and permission-controlled tools.

**Status:** native Linux/X11 development verification and Windows-targeted packaging. See [the implementation report](docs/IMPLEMENTATION_REPORT.md) for current features, actual tests, measurements and platform limits, and [the full acceptance checklist](docs/SPEC.md) for broader release work. A successful build does not establish installer acceptance.

## Development

Install Node.js 22.13+ (24 recommended), Rust and the platform's Tauri development prerequisites. Windows requires the MSVC toolchain, Visual Studio C++ Build Tools and Microsoft Edge WebView2. Linux requires WebKitGTK/GTK development files, Secret Service/D-Bus with `libdbus-1-dev`, and the [native capture/input dependencies](docs/AUTOMATION.md#platforms-and-development-dependencies). Match the NVIDIA driver to your selected CUDA runtime; CPU fallback is also available.

```sh
npm ci
npm run tauri dev
```

`npm run dev` starts a browser preview only. Model loading, persistence, credentials and tool execution require the native application.

## Load the model

In **Models & runtime → Model files**, use **Download model** and **Install CUDA runtime** for managed setup. The app reports required disk space, verifies pinned SHA-256 hashes and supports cancellation. After each operation completes, select **Use verified model** or **Use installed runtime**, save settings, then load the model. Existing managed model files are verified rather than overwritten; runtime installations use new directories. Full model network-transfer and crash-recovery acceptance remain outstanding.

For development, the preparation script is also available:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/prepare-runtime.ps1 -IncludeModel
```

This downloads pinned llama.cpp archives and the approximately 2.07 GB GGUF into `.local/`, verifies SHA-256 hashes, and extracts the runtime. Downloads and extracted files need additional disk space. Omit `-IncludeModel` to prepare only the runtime.

In **Models & runtime**, choose absolute paths to the runtime executable (`llama-server` on Linux, `llama-server.exe` on Windows) and a compatible GGUF, save, then select **Load model**. Changes to load settings take effect after unloading and loading again. The page distinguishes saved settings from the loaded configuration. **Diagnostics → Retest tool calling** sends a real structured diagnostic request; unsupported results keep text chat available.

GPU layers `-1` requests all supported layers; `0` uses CPU layers. Context length, CPU threads, batch sizes, Flash Attention and K/V cache precision/placement control resource use. GPU telemetry is device-wide and may include other applications. See [runtime provenance](docs/RUNTIME.md) and [context limits](docs/CONTEXT.md).

## API providers

In **Models & runtime → API providers**, choose **Add provider**. Pick an OpenRouter, Groq or Together shortcut, or choose **Custom provider** and enter a name, API format, base URL and API key. Every URL remains editable. Formats include OpenAI-compatible Chat Completions, native OpenAI Responses and native Claude Messages. Hosted APIs require HTTPS; custom OpenAI endpoints may omit a key when the server supports it. Claude endpoints require one. Use the provider's API root, including any prefix (for example `https://openrouter.ai/api/v1`), without `/chat/completions`, `/responses` or `/messages`. See [native Responses details and acceptance status](docs/NATIVE_RESPONSES.md).

Use **Save & test connection** to discover models and import reported context/output limits and capabilities. Search large catalogs by model ID. If a model has no reported context capacity, set **Context tokens** under **Edit**, then choose **Use**. Manual model IDs and limits survive catalog refreshes. If the endpoint does not list models, enter a model ID manually and use **Test inference**, which sends a small test request and may incur a provider charge. **Save provider** stores the connection without a network request. Saving and testing require the desktop app.

Keys are encrypted on your device with the encryption key protected by the OS credential store (on Linux, unlock GNOME Keyring or KWallet). Leaving the key blank while editing preserves it; entering a new key replaces it. Changing the URL or API format requires an explicit key for that destination. **Delete API key** removes just the credential and requires testing again; provider settings and chat history remain. **Remove** deletes the connection and its key. Chats that used a removed provider retain their history and require an explicit model choice before continuing.

The local model remains the default. Selecting an API model sends the conversation to the configured service.

## Workspace panels

Open the workspace panel to use **Review**, **Terminal**, **Browser**, or **Editor** (formerly Files). Editor includes a folder explorer, new files/folders, rename/delete, syntax highlighting, find/replace, undo/redo and Ctrl+S. Drafts stay open across tabs and Settings; closing an edited document or the app offers Save, Discard and Cancel. Saves check for external changes and replace the file atomically. UTF-8 text files up to 2 MiB are supported. See [workspace behavior and verification](docs/WORKSPACE.md).

## Connectors, tools and skills

Local web grounding is available through the Research preset's `web_search` tool. **Settings → Web Search** configures self-hosted SearXNG, JSON health checks, Fast/Balanced/Deep budgets and cache clearing. The shared pipeline uses bounded evidence and citations, supports HTML/PDF text/JSON/CSV/TSV, and returns attributed extracts when local synthesis is unavailable. See [setup and diagnostic commands](docs/WEB_SEARCH_PHASE1.md).

The `get_weather` tool retrieves [Open-Meteo](https://open-meteo.com/en/docs) current, hourly and daily forecasts directly, without SearXNG or a model call. It is offered in Standard, Research, Code, Coding and Creator. In **Weather**, enable it and save a location and Celsius/Fahrenheit preference. Pass `location` when no preference exists and optional `forecastDays` from 1 to 7 (default 3). Results include resolved place/timezone, conditions, feels-like temperature, humidity, wind, rain probability, source and original fetched time. A persistent 10-minute cache avoids repeat requests. Node/network are required; offline mode blocks retrieval, ambiguity returns choices and no device location is inferred. Normal tool approval applies; the legacy `weather` name remains compatible.

For current system time, the existing shell command tool can run `date '+%Y-%m-%d %H:%M:%S %Z (%z)'` on Linux or macOS (`TZ=Asia/Kolkata date` selects a timezone). No additional clock tool is needed; existing `system_time` support is unchanged.

Connectors includes 14 TrueForge presets. Connect and authenticate a service, then select individual discovered tools above the chat. Account-specific services require your credentials. Public DeepWiki calls have native end-to-end coverage; this does not prove every authenticated service works with your account.

Choose a workspace folder to enable file tools. **Bash** and **Python** settings independently configure enablement, actual executable detection/testing, cwd, timeout, environment, approval and optional Unix memory limits; Python can reuse a project venv. Python files/snippets support args, stdin, imports and declared output artifacts with text/image previews. **Execution** retains Node.js and PowerShell configuration. Workspace Terminal opens a manual persistent PTY. Local code runs with your account's filesystem/network permissions; workspace checks are not an OS sandbox.

Every conversation has **Ask for approval**, **Auto-approve reads** and **Full access** modes. With no remembered preference, new chats default to Ask. Auto-approve permits known workspace reads; other tools ask. Full access runs selected enabled tools automatically, while a stricter Bash/Python “always ask” setting still requires approval. Decisions and results remain in tool history. **Permission rules** let you settle recurring cases once — “always allow `cargo test`”, “never allow writes outside `src/`” — and are evaluated before the access mode. See [permissions](docs/PERMISSIONS.md).

**Review** supports native Git status/diffs, file stage/unstage, commits with explicit staged-content confirmation, branch creation/switching and history. **Git** settings opens the current repository review. Measured responses offer optional **Response diagnostics** with backend generation/prompt rates, cache/model details, first streamed token, first visible answer, total duration and recorded phase timings. The first streamed token can be reasoning or a tool token; first visible answer is measured separately, before UI rendering. Missing historical/provider data stays unavailable.

The harness preserves rendered conversation history for prompt reuse, shares repeated tokenizer requests, and reports provider-measured cache usage in the context popover. Chat and child agents share validated streaming tool protocols. See [cache behavior, tool contracts and verification](docs/CACHE_AND_TOOLS.md).

Coding workflows also include stdio DAP debugging, language-server code-action proposals, source-bound test evidence, change-impact searches, HTTP sessions and OpenAPI inspection, SQLite query/migration rehearsal, and repeatable benchmark/profile inspection. Tools follow the selected workspace, execution permissions and capability controls; larger catalogs use tool discovery. See [development tools, examples and limits](docs/DEVELOPMENT_TOOLS.md).

The harness reaches the model through ordinary tools: a tolerant file editor, an automatic formatter, code intelligence, a debugger, verification and profiling, a dependency-aware task graph, structured review findings, workspace snapshots for undo, plugin tools you author, and the workspace's own `AGENTS.md` rules. See [harness tools](docs/HARNESS_TOOLS.md).

Skills installs 13 pinned packages. Active skills supply instructions and a tool for reading supporting files. **Check dependencies** reports connector/interpreter/CLI status with repair guidance, and the **Version** panel reports installed versus pinned revisions with integrity results; reinstalling verifies every file before publishing and never executes package scripts. Skill scripts themselves run only through a separately enabled execution tool after approval. See [skills](docs/SKILLS.md).

Browser and computer automation have separate **Settings → Browser Use** and **Settings → Computer Use** tabs. Set up the pinned Playwright engine, enable a capability, and select it in the chat’s Tools panel. Desktop input supports Windows and Linux X11 and requires a vision-capable model. See [setup, permissions, open-source provenance, and tested limitations](docs/AUTOMATION.md).

## Response controls and outputs

**Reasoning → Auto / Quick / Think** is available beside the composer when the selected model has a known control. The saved default lives in **Settings → Models & generation → Inference**; the composer overrides it for the next response. Auto favors a direct local reply when no tools are offered; otherwise it follows the model default. Quick reduces supported reasoning and optional tool-catalog overhead; selected workspace, execution and connector tools remain available. Think requests more effort where supported. Maximum response tokens is a separate control and includes reasoning plus the answer. Unknown models and private subscription protocols keep their defaults rather than receiving guessed API fields. Changing reasoning or catalog selection can invalidate a warm prompt prefix; schemas stay stable within a turn.

Answers lead the conversation. Reasoning and completed activity are collapsed, while pending approvals, errors, Stop and optional Pause remain available. Sources use stored titles, links, citation IDs and metadata; no dates or timing estimates are invented. Generated files share a viewer for text/code, Markdown, images and CSV/TSV tables, with Preview/Source, copy, save, open and close actions. HTML remains literal source. More than 16 files are disclosed with **Show all files**. Native Save uses a file dialog and atomic copy; native Open uses an in-app expanded viewer.

**Generative UI** adds Auto, Text and Visual response modes to the existing chat using the official [OpenUI Lang](https://github.com/thesysdev/openui) renderer. Set the application default in Models & runtime settings. The home and chat composers intentionally have no response-mode selector. Visual answers can include charts, diagrams, quizzes, local simulations and coding views, with interactive state restored from SQLite. Formatting runs after the ordinary agent answer without tool authority; invalid output falls back to that original answer. Small local models may require bounded syntax repair or text fallback. See [generative UI](docs/GENUI.md) and [verification/measurements](docs/GENERATIVE_UI_IMPLEMENTATION.md).

**Ctrl+Shift+P** or **File → Commands** opens the keyboard command palette. Existing Ctrl+K conversation search, Ctrl+N new conversation and Ctrl+, Settings shortcuts remain. Settings are grouped into General & appearance, Models & generation, Tools & execution, Privacy & permissions, Integrations and Diagnostics; language and Weather preferences remain searchable.

**General → Project documents** explicitly enables local keyword retrieval for selected project files: up to 16 UTF-8 Markdown, text, RST, CSV or TSV documents, each at most 32 KiB. Files are re-read for current contents; changed/deleted files do not leave a stale corpus. Disable deletes the file selection, and removing the project forgets its selection. Excerpts already included in saved conversation/model context remain with that history. Selected excerpts go to the API endpoint when an API model is used. This is bounded local retrieval, not a background repository crawler or embedding service.

**Voice** starts only after an explicit microphone action and adds the transcript to the draft for review. Browser speech services may send audio to their provider; the UI explains this before starting. Unsupported desktop webviews show an unavailable state. Cancel, submission and conversation changes stop the session and discard late results. No local transcription engine is bundled.

**Diagnostics → Response latency** runs bounded fixtures on the loaded local model with synthetic tool schemas on/off and requested cold/prime/warm caches. It executes no tools or generated code and uses supplied fictional research evidence rather than live web search. Median/p95 timings are shown alongside strict fixture correctness and incomplete-stream counts; incomplete streams are excluded from latency aggregates. Reports can be loaded and saved. The repeatable standalone matrix and measurement limits are documented in [response benchmarks](docs/RESPONSE_LATENCY_BENCHMARK.md). Small samples establish no causal speedup. Current implementation and verification evidence is in the [review checklist](docs/REVIEW_REMEDIATION_2026-10-17.md).

## Data and privacy

Application data includes `locallm.sqlite`, skills, encrypted credential files and `runtime.log`. Typical locations are `%APPDATA%/app.locallm.desktop` on Windows and `$XDG_DATA_HOME/app.locallm.desktop` (normally `~/.local/share/app.locallm.desktop`) on Linux. Windows Credential Manager, Linux Secret Service or macOS Keychain protects the encryption key; the OS store must be available/unlocked. Conversations and tool audits are not encrypted by the application. Unsent drafts use local WebView storage.

Local inference uses an authenticated loopback endpoint. Explicitly selected API providers receive conversation data at their configured endpoints. Connector calls and skill downloads use the network. External tools receive the data supplied in their arguments; local code can also access the network. See [security boundaries](docs/SECURITY.md) and [export contents](docs/EXPORTS.md). Close the app before copying its database for backup, preserving any companion SQLite files.

## Checks and packaging

```powershell
npm test
npm run typecheck
npm run build
npx playwright test
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run tauri build
```

The configured Windows target is NSIS under `src-tauri/target/release/bundle/nsis/`. Model weights and llama.cpp are currently prepared separately, not bundled. Native smoke scripts exercise the debug application; release install/uninstall testing is a separate requirement.

## Troubleshooting

- **Load fails:** check paths, NVIDIA driver and `runtime.log`. Reduce GPU layers or context size when memory is insufficient.
- **Context overflow:** shorten the prompt, select fewer tools/skills, increase loaded context within available memory, or start a new chat. History is not silently truncated.
- **Response token limit:** increase Maximum response tokens or request a shorter answer. Partial output and its error remain saved; unfinished tool calls are not executed.
- **Tool denied:** denial prevents further tool calls for that turn. Change permissions before sending a new message if needed.
- **Skill integrity failure:** remove and reinstall the affected package.

Remaining release verification includes full managed-model transfer/crash recovery, authenticated account and representative skill workflows, Windows/macOS native acceptance and install/uninstall/signing. Preview limits, Wayland restrictions, test skips and current bundle advisories are explicit in [the implementation report](docs/IMPLEMENTATION_REPORT.md). Installer provenance and third-party notices are in [release notes](docs/RELEASE.md); [desktop lifecycle](docs/DESKTOP.md) describes launch behavior.
