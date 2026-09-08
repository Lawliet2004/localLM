# Windows packaging evidence

Internal development build, 2026-09-08. This is not a production release: the remaining product and verification requirements in SPEC.md still apply.

## Artifact

- Source application commit: `8e36be7`.
- Command: `npm run tauri build`.
- Target: Windows x64, NSIS, current-user installation.
- Installer: `src-tauri/target/release/bundle/nsis/LocalLM_0.1.0_x64-setup.exe`.
- Installer size: 3,412,952 bytes.
- SHA-256: `88533c3db295fcd7797b0b1e842ed33744e3d9dfb5ff7261b2b8c4d682ea67cb`.
- Application size: 10,120,704 bytes.
- Authenticode status: `NotSigned`.
- Initial optimized build: 6 minutes 18 seconds on the development machine.

The installer includes the application and bundled frontend. It does not include model weights or the inference runtime. Existing separately prepared files were used for the installed-app inference test. No updater or signing pipeline has been established.

## Verified on the development machine

`scripts/installer-smoke.ps1` installs silently into `.local/installer-smoke`, launches the installed application, runs `scripts/release-smoke.mjs`, closes it, and uninstalls it. It refuses an existing current-user LocalLM registration or an occupied test directory. Run it only with LocalLM closed and the pinned runtime/model configured in the existing profile.

The installed app served the bundled frontend at `http://tauri.localhost/`, loaded the real local model, answered `17 + 25` with `42`, and saved two messages with a completed assistant response. No JavaScript errors were observed during that flow. Generation preferences were restored and the temporary conversation was deleted. A screenshot was inspected for visual correctness.

Both installer and uninstaller launcher returned zero. The test waited for the installation folder and uninstall registration to disappear, then verified the SQLite database hash was unchanged by uninstallation. NSIS's initial uninstaller process hands off to a temporary process; waiting for the launcher alone is insufficient. The test does not select app-data deletion.

Evidence files are generated under ignored `test-results/`: `installer-smoke.json`, `release-smoke.json`, and `installed-release.png`. The installer build is also ignored. Rebuilding may produce a different hash; record new measurements rather than treating this manifest as proof of a later artifact.

## Remaining release checks

Clean-profile onboarding, machines without WebView2/runtime/model, upgrade/rollback behavior, interactive installer pages, signing, third-party notices, account-specific connectors, and the rest of the full acceptance checklist remain unverified or incomplete. The existing profile used here is not evidence that a new user can complete setup without developer assistance.

The production npm dependency audit (`npm audit --omit=dev --json`) reported zero known vulnerabilities on this date. That result is limited to the audited npm dependencies; it is not a complete security audit of the application or native dependencies.
