# LocalLM

## Objective
Ship a polished Windows desktop agent harness using Rust, TypeScript, React and Tauri 2. MiniCPM5-2B Q6_K runs locally using the RTX 2050. Preserve the complete requested scope: ordinary chat, all TrueForge connector and skill presets, execution, runtime tuning, and an installable release.

## Architecture
- React renders a restrained, keyboard-accessible workspace: conversation sidebar, chat, connectors, skills, models, settings and optional detail panel.
- Rust owns SQLite persistence, credentials, runtime lifecycle, networking, agent execution and permissions. Web content cannot invoke arbitrary shell commands.
- A pinned llama.cpp binary runs as a managed child process on loopback with a random API key. CUDA is preferred, CPU fallback is explicit. Do not download model weights until provenance and storage paths are recorded.
- MCP connectors use remote HTTP with OAuth/PKCE or bearer credentials, plus configurable local stdio servers. Tools are discovered on demand. Secrets stay outside model context.
- Skills install pinned upstream instruction packs with provenance and dependencies. Instructions do not confer execution permissions.
- Code execution supports Daytona and an explicit local execution mode. Local processes must not be represented as an isolation boundary.
- Initial target: Windows x64, 4 GB NVIDIA VRAM, 16 GB system RAM. Other operating systems require separate release verification.

## Commands
- `npm run dev`: frontend development server.
- `npm run tauri dev`: native desktop development.
- `npm run build`: TypeScript check and optimized frontend build.
- `npm test`: frontend behavior tests.
- `cargo test --manifest-path src-tauri/Cargo.toml`: native unit/integration tests.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`: Rust lint.
- `npm run tauri build`: Windows distribution.

## Structure and style
`src/components/` contains focused UI components; `src/lib/` contains typed IPC contracts and presentation logic. `src-tauri/src/` contains native services with colocated tests. `catalog/` contains versioned upstream presets; `scripts/` contains reproducible runtime preparation and release checks. `docs/` records evidence, design and remaining work.

Use named types, explicit errors and small functions. Example: `async function loadModel(config: RuntimeConfig): Promise<RuntimeStatus>`. No fake connection success, fabricated metrics or silent failures.

## Acceptance checklist
- [x] Native application starts, closes cleanly, remembers window state, and packages into a Windows installer. Evidence: docs/DESKTOP.md and docs/RELEASE.md; broader release verification remains below.
- [ ] Persistent conversations: create, rename, delete, search, stream, cancel, retry, copy and export; recover interrupted turns.
- [ ] Verified MiniCPM5 Q6_K chat template, reasoning separation, XML/structured tool-call parsing, tool result continuation and bounded agent loops.
- [ ] GPU detection and actual offload reporting; load/unload/reload; CPU fallback; context, GPU layers, CPU threads, K/V cache precision/placement, Flash Attention, batch sizes and generation parameters.
- [ ] Runtime download verification, model import/download progress, cancellation, disk-space checks, useful load errors, logs and memory/performance measurement.
- [ ] All 14 presets: Linear, Notion, Sentry, DeepWiki, Exa, Parallel Web, GitHub, Tavily, Bright Data, Supabase, Stripe, Confluence, Jira, PostHog. Connection test, tool discovery, auth refresh/disconnect, per-chat selection and actual tool calls.
- [ ] All 13 skills: algorithmic-art, skill-creator, mcp-builder, web-artifacts-builder, tavily-research, supabase, wiki-architect, wiki-qa, linear, gh-fix-ci, notion-knowledge-capture, sentry, jupyter-notebook. Install/remove/update, pinned provenance, inspect content, dependency status, activation and real execution workflows.
- [ ] Local and Daytona execution with timeouts, cancellation, bounded output, workspace restrictions and the selected permission policy; file and artifact previews.
- [x] Per-conversation Ask for approval, Auto-approve reads and Full access modes, with visible scope, durable settings and audited authorization.
- [ ] Credential protection, IPC input validation, CSP, hostile content handling, permission boundaries and no secrets in logs or prompts.
- [ ] Polished light/dark interface, keyboard navigation, responsive layout, useful empty/loading/error states, readable Markdown/code/tables.
- [ ] Automated unit/integration tests plus browser and native acceptance tests; GPU benchmark and release install/uninstall verification recorded.
- [ ] Documentation: setup, credentials, data locations, runtime options, privacy, troubleshooting, third-party notices and release limitations.

## Verification and boundaries
Use actual SQLite, local HTTP test servers and temporary directories for integration tests. Test malformed/truncated streams, cancellation, startup failures, context overflow and tool failures. Real service credentials are needed for account-specific end-to-end evidence; never claim fixtures prove those accounts work.

Always validate inputs, retain error evidence, run relevant checks before commits and keep the full checklist intact. User authorization covers ordinary implementation, dependencies, local setup and build work. Ask only when an external account action or missing preference is necessary. Never commit credentials, expose the runtime on the network, silently execute model-generated code, or declare production readiness from a successful compile alone.

## Execution order
1. Scaffold and verify toolchain; obtain a pinned runtime and test Q6_K inference/tool use.
2. Build typed runtime configuration and SQLite conversation services with failure tests.
3. Connect the native chat UI, streaming and runtime management.
4. Implement MCP transport/auth and the connector catalog; then the bounded agent loop.
5. Add skill installation, execution providers, permissions and artifacts.
6. Test UI, real native flows and GPU behavior; harden recovery and performance.
7. Build installer, audit every acceptance item and document remaining external dependencies.
