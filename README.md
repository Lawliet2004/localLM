# LocalLM

A local AI desktop workspace built with Rust, TypeScript, React and Tauri 2. MiniCPM5-2B Q6_K runs through a managed llama.cpp process, with GPU offloading, chat, MCP connectors, skills and permission-controlled tools.

**Status:** native Windows application under active development. See [the full acceptance checklist](docs/SPEC.md). A successful installer build does not establish production readiness.

## Development

Install Node.js, Rust with the MSVC toolchain, Visual Studio C++ Build Tools and Microsoft Edge WebView2. The tested environment uses Node 24 and Rust 1.98. The pinned GPU runtime requires an NVIDIA driver compatible with CUDA 12.4.

```powershell
npm ci
npm run tauri dev
```

`npm run dev` starts a browser preview only. Model loading, persistence, credentials and tool execution require the native application.

## Load the model

```powershell
powershell -ExecutionPolicy Bypass -File scripts/prepare-runtime.ps1 -IncludeModel
```

This downloads pinned llama.cpp archives and the approximately 2.07 GB GGUF into `.local/`, verifies SHA-256 hashes, and extracts the runtime. Downloads and extracted files need additional disk space. Omit `-IncludeModel` to prepare only the runtime.

In **Models & runtime**, choose the absolute paths to `.local/runtime/llama-server.exe` and `.local/models/MiniCPM5-2B.Q6_K.gguf`, save settings, then select **Load model**. Changes to load settings take effect after unloading and loading again. The page distinguishes saved settings from the loaded configuration.

GPU layers `-1` requests all supported layers; `0` uses CPU layers. Context length, CPU threads, batch sizes, Flash Attention and K/V cache precision/placement control resource use. GPU telemetry is device-wide and may include other applications. See [runtime provenance](docs/RUNTIME.md) and [context limits](docs/CONTEXT.md).

## Connectors, tools and skills

Connectors includes 14 TrueForge presets. Connect and authenticate a service, then select individual discovered tools above the chat. Account-specific services require your credentials. Public DeepWiki calls have native end-to-end coverage; this does not prove every authenticated service works with your account.

Choose a workspace folder to enable file tools. Configure an installed Python, Node.js or PowerShell executable in Execution to enable local code. Local code runs with your account's filesystem and network permissions and is not sandboxed.

Every conversation has **Ask for approval**, **Auto-approve reads** and **Full access** modes. New chats default to Ask. Auto-approve permits known workspace reads; other tools ask. Full access runs enabled tools without prompts. Decisions and results remain in tool history. See [permissions](docs/PERMISSIONS.md).

Skills installs 13 pinned packages. Active skills supply instructions and a tool for reading supporting files. Dedicated script execution, dependency management and updates remain incomplete. See [skills](docs/SKILLS.md).

## Data and privacy

On Windows, application data normally resides in `%APPDATA%/app.locallm.desktop`: `locallm.sqlite`, installed skills, encrypted credential files and `runtime.log`. Windows Credential Manager protects the credential encryption key. Conversations and tool audits are not encrypted by the application. Unsent drafts use local WebView storage.

Inference uses an authenticated loopback endpoint. Connector calls and skill downloads use the network. External tools receive the data supplied in their arguments; local code can also access the network. See [security boundaries](docs/SECURITY.md) and [export contents](docs/EXPORTS.md). Close the app before copying its database for backup, preserving any companion SQLite files.

## Checks and packaging

```powershell
npm test
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

Outstanding work includes in-app verified downloads, Daytona, custom stdio connectors, skill execution workflows, existing-file editing/artifact previews, retry/compaction, third-party notices and full release verification. [Progress](docs/PROGRESS.md) records evidence and limitations; [desktop lifecycle](docs/DESKTOP.md) describes launch behavior.
