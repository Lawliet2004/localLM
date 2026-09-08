# Evidence and remaining work

## 2026-09-08
- Empty workspace inspected. Scaffolded official Tauri React/TypeScript template.
- Rust 1.98.1, Node 24.14.1 and npm 11.11.0 available.
- Prior hardware probe: RTX 2050 4096 MiB, Ryzen 5 7535HS, 15.2 GiB usable RAM.
- All acceptance items in SPEC.md remain unverified. Application is not release-ready.

## Verified first native chat slice
- `npm test`: 3 runtime form behavior tests passed.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: 13 tests passed (runtime validation, GGUF rejection, SQLite persistence/recovery, SSE boundaries).
- `npm run build`: passed; initial application JS approximately 386 kB (119 kB gzip).
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`: passed before MCP dependency work.
- `npx playwright test`: 2 browser tests passed; inspected dark-theme screenshot. Theme switch, catalog filtering and no horizontal overflow at 320/768/1024/1440 verified. This does not verify full accessibility.
- `node scripts/native-smoke.mjs`: actual Tauri IPC -> Rust -> llama.cpp -> RTX 2050 -> streamed answer `42`, then conversation rename and persistence after webview reload. Model load 6554 ms; end-to-end short generation and UI checks approximately 2238 ms. No captured webview errors.
- Runtime and Q6_K downloads completed with pinned SHA-256 checks. Runtime `--list-devices` reported CUDA0 RTX 2050, 4095 MiB total / 3299 MiB free before model load.
- `node scripts/runtime-probe.mjs`: real MiniCPM Q6_K structured `get_weather({city:"Paris"})` followed by synthetic tool result continuation; model incorporated 19 C. Reported short follow-up generation rate 49.8 tokens/s. This is not a full benchmark or live connector test.

## Still required
All full acceptance items in SPEC.md remain open. Connector connection controls are wired to native MCP and OAuth services; skill installation remains unavailable. Next: bounded agent tool loop and approval UI, authenticated-provider verification, then skills/execution. Chat still needs retry, context handling, durable error details, restart acceptance, export verification, broader cancellation/failure tests and live runtime metrics. Model settings need presets, download management, richer GGUF validation and actual placement reporting. No installer/release verification yet.

## Development processes
Vite was started on 127.0.0.1:1420. A debug native app was launched with WebView2 CDP on 9223 for smoke tests; close that debug process before rebuilding the executable. Production launch does not enable CDP. Runtime has been unloaded after the smoke test.

## Verified connector discovery slice
- Native connector UI exposes all 14 presets, API token entry, OAuth sign-in/cancel, connection/disconnection, credential removal and tool schemas.
- `cargo test`: 18 passed. `cargo clippy --all-targets -- -D warnings`: passed after MCP/OAuth/vault integration.
- `npm run build`, 3 frontend tests and 2 browser tests passed.
- `node scripts/connector-smoke.mjs`: real native application connected to public DeepWiki MCP, discovered ask_question/read_wiki_contents/read_wiki_structure, and disconnected. Native screenshot inspected.
- Credentials use AES-256-GCM encrypted files with a Windows Credential Manager master key; cryptographic tampering and large-secret tests passed. Actual account sign-in, refresh/revocation, and OS vault restart tests remain required.
- This verifies discovery only, not chat tool execution or all external services.
## Verified first agent tool loop
- Streamed tool calls assemble across chunks, with up to 8 calls per round, 8 execution rounds, bounded arguments and results, and a maximum of 32 offered tools.
- Connected services can be selected in chat. Each proposed call opens a modal showing the service, real tool name and exact arguments, with single-use Allow once/Deny decisions. Escape denies. Cancellation removes pending approval without executing it.
- Requests/results are persisted as tool messages and rendered in expandable cards. Remote timeout/cancellation reports an unknown outcome and does not automatically retry.
- `node scripts/agent-smoke.mjs`: real MiniCPM Q6_K + native Rust MCP + DeepWiki read_wiki_structure for tauri-apps/tauri. Allowed call returned repository sections (verified in stored JSON); denied call produced a denied record and explanation; cancellation during approval ended with no executed tool record. Three scenarios passed twice after replacing opaque aliases with descriptive tool names.
- `cargo test`: 21 tests passed, including interleaved/malformed tool fragments and single-use approval identity. Frontend production build passed.
- Remaining agent work: individual tool selection for large catalogs, schema validation, context/token budgeting, durable pending-approval recovery, live tool progress, richer error states, full tool history replay, and broader transport failure/hostile-output tests. This slice is not production readiness.
