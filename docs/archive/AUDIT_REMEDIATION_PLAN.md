# Full audit remediation plan

Scope: resolve all 24 findings in [the audit](AUDIT_2026-10-16.md), preserve existing workspace changes, and repair the failing checks identified during verification. Implemented collaboratively by Codex and SWE-2 Max through Devin CLI. SWE-2 Max supplied the sandbox/artifact and Sidebar/Trajectory batches; Codex integrated them, added atomic no-follow file access, and completed the application lifecycle and permission fixes.

## Implementation order

1. **Security and integrity:** A01/A03/A04 workspace and network boundaries; A02 project identity; A09 historical background results; A10/A11 effective tool permissions and text-only providers; A17 offline retrieval; A18 artifact budgets; A22 dependency advisory.
2. **Turn lifecycle:** A06/A12 send acceptance and input preservation; A07 task workspace consistency; A13 current-project shortcuts; A15 authoritative pause state; A16 exact preflight payload; A23 final-response round reservation; A24 cancellation during preparation.
3. **UI resilience:** A05 storage failure; A08/A20 trajectory ownership and pagination; A14 keyboard project controls; A19 automation setup routes; A21 model-picker dismissal and focus.
4. **Verification:** targeted regressions, then TypeScript/build, Vitest, native tests/Clippy, worker/transport tests, browser workflows, dependency audit, and bounded desktop/narrow UI inspection. Record any environment-only limits separately.

## Finding status

| Finding | Status | Verification |
| --- | --- | --- |
| A01 Workspace search boundary | Fixed | Symlink/credential regressions |
| A02 Project identity collision | Fixed | Same-name projects retain task roots |
| A03 DNS-pinned fetch | Fixed | Resolver/redirect boundary tests |
| A04 Bounded reads | Fixed | Large file and streamed response limits |
| A05 Storage failure resilience | Fixed | Throwing Storage regression |
| A06 Rejected send preserves input | Fixed | Missing-project draft/attachment regression |
| A07 Task workspace consistency | Fixed | Folder selection and send use same root |
| A08 Trajectory request ownership | Fixed | Conversation switch during delayed load |
| A09 Background history replay | Fixed | New and legacy settlement replay |
| A10 Tool source policy | Fixed | Inventory and dispatch gates |
| A11 Text-only providers | Fixed | Zero-tool inventory regression |
| A12 Accepted send avoids duplicates | Fixed | Persisted prompt plus inference failure |
| A13 Project-aware shortcut | Fixed | Project switch and all-projects shortcut |
| A14 Keyboard project actions | Fixed | Keyboard disclosure/action access |
| A15 Pause lifecycle | Fixed | Failure rollback and conversation switching |
| A16 Preflight payload/freshness | Fixed | Attachments/model and delayed response tests |
| A17 Offline retrieval policy | Fixed | Every network retrieval entry point |
| A18 Artifact byte budget | Fixed | Unicode/nested/error/scalar result limits |
| A19 Automation setup navigation | Fixed | Browser/computer destinations |
| A20 Trajectory pagination | Fixed | More than 1,000 events accessible |
| A21 Model picker keyboard/focus | Fixed | Escape/outside dismissal and focus |
| A22 Vulnerable dependency | Fixed | Patched lockfile and npm audit |
| A23 Final-response round budget | Fixed | Last round is tool-free |
| A24 Preparation cancellation | Fixed | Stop/Pause before and during preparation |

## Additional integration finding — A25

Legacy HTML extraction leaked allocated strings with `Box::leak` on every removed script/style block. Unicode lowercasing also changed byte offsets: `İ<script>hidden</script>` reproducibly panicked with “start byte index 26 is out of bounds for string of length 25.” The extractor now uses owned buffers, linear scans per tag family, and ASCII case matching that preserves UTF-8 offsets. A regression covers Unicode, uppercase tags, repeated blocks, and similarly named custom tags. This is fixed in addition to the original 24 findings.

## Verification results

- `npm test`: **490 passed, 1 optional live MiniCPM test skipped**. The test pool is bounded to four workers to avoid CPU/memory contention; large trajectory pagination runs in the real browser.
- `npx playwright test --workers=4`: **33 passed**, including the 1,200-event pagination and stale refresh regressions.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: **409 passed, 5 opt-in checks ignored**.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`: **passed**.
- `npm run build`: **passed**. The existing bundle-size advisory remains an optimization opportunity.
- `npm run test:web-transport`: **19 passed** using the portable TypeScript test loader.
- SearXNG/weather worker tests: **7 passed**.
- `npm audit`: **0 vulnerabilities**, including development dependencies; `source-map-js` upgraded to 1.2.2.
- Native WebKitGTK layout check: **passed** for navigation, hide/restore, tab changes, resize, and close on an isolated Xvfb display. The Xvfb package was extracted into ignored QA storage without installing system packages.
- Desktop and 390px UI inspection: no horizontal overflow or page errors; model-picker Escape dismissal and focus verified. The detector only reports the existing live-activity text gradient, retained as the established state cue.
- `git diff --check`: **passed**; the local code graph was refreshed.

QA logs, fixtures, screenshots, collaboration exports, and the before-change file hash inventory are in `.local/qa/remediation-2026-10-16/` (ignored by Git). New permanent regression coverage is in the component/native tests and `e2e/audit-regressions.spec.ts`.

Five native tests remain opt-in: real browser automation with its external MCP package, isolated desktop automation, live Hugging Face access, Windows CUDA runtime flags, and pinned CUDA archive extraction. Windows packaging, GPU inference, and authenticated external services were not exercised by these local checks.

## Check failures found by the audit

- **Fixed:** portable encrypted vault test setup and interpreter-dependent native tests; production keyring protection is unchanged.
- **Fixed:** native Clippy errors, with runtime safeguards preserved.
- **Fixed:** transport tests run on Node builds without built-in TypeScript stripping.
- **Verified:** native window layout using an isolated virtual display.
