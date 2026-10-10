# LocalLM — agent notes

## Verify UI changes
- `npm run typecheck` — tsc for app + e2e
- `npm run lint` — oxlint, warnings fail
- `npm test` — vitest (jsdom)
- `npx playwright test` — e2e against the Vite dev server on :1420 (Edge channel)

## Conventions
- Design system and UI patterns: see `DESIGN.md`. Product truths: `PRODUCT.md`.
- Settings navigation uses `Page = 'chat' | SettingsSection` (`src/components/Sidebar.tsx`). A settings section takes over the window through `SettingsShell` (`src/components/Settings.tsx`).
- The `browser` harness tool (`src-tauri/src/browser_agent.rs` + in-page driver `browser_agent.js`) drives the workspace "agent" browser tab. Settings → Browser (`browser` setting: mode off/auto/preferred, maxActions per reply) gates it in `chat.rs` and `harness.rs`.
- e2e fixtures that mock `src/lib/api.ts` must stub every API that is called on mount (e.g. `browserCloseAll`).
- On Windows, don't round-trip source files through PowerShell `Get-Content`/`Set-Content`: it breaks UTF-8 (`·` → `Â·`) and adds a BOM. Use Node or the editor tools instead.
