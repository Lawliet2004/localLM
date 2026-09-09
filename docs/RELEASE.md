# Windows packaging evidence

Current development build, 2026-09-09. This is not a production release: the remaining product and verification requirements in SPEC.md still apply. No signing pipeline has been established.

## Current artifact (HEAD `57e0806`)

- Source application commit: `57e0806` (`Harden native chat smoke for reruns`).
- Command: `npm run tauri build` (includes `tsc && vite build`, release Rust build, NSIS bundling; completed in about 7m14s on the development machine).
- Target: Windows x64, NSIS, current-user installation.
- Installer: `src-tauri/target/release/bundle/nsis/LocalLM_0.1.0_x64-setup.exe`.
- Installer size: 3,678,128 bytes; mtime 2026-09-09T19:52:24+05:30.
- Installer SHA-256: `61E72A40B7352A0A517AAACD7A790E3BE7B5EF86B566BF40F9745251FAE52E16`.
- Application: `src-tauri/target/release/locallm.exe`, 10,988,544 bytes.
- Application SHA-256: `2C16210CFC1E16323387DE19A6351F46A36DBFC3E753D32862B4115D2BC32D7F`.
- Authenticode status: `NotSigned`.
- Frontend bundle at build time: `dist/assets/index-nNk_mJM7.js` (436.15 kB, 130.95 kB gzip), `dist/assets/index-DjvFRbD0.css` (19.45 kB), `dist/index.html` (0.52 kB).

The installer includes the application and bundled frontend. It does not include model weights or the inference runtime. Existing separately prepared files were used for the installed-app inference test described below. No updater pipeline has been established.

## Earlier artifact (2026-09-08, superseded by the current build above)

### Artifact

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

## Verified on the development machine (current build, 2026-09-09)

`scripts/installer-smoke.ps1` installs silently into `.local/installer-smoke`, launches the installed application, runs `scripts/release-smoke.mjs`, closes it, and uninstalls it. It refuses an existing current-user LocalLM registration or an occupied test directory. Run it only with LocalLM closed and the pinned runtime/model configured in the existing profile.

The installed app served the bundled frontend at `http://tauri.localhost/`, loaded the real local model, answered `17 + 25` with `42`, and saved two messages with a completed assistant response. No JavaScript errors were observed during that flow. Generation preferences were restored and the temporary conversation was deleted. A screenshot was inspected for visual correctness. This run used the current 2026-09-09 installer (`installer-smoke.json`, `release-smoke.json`, `installed-release.png` under ignored `test-results/`).

Both installer and uninstaller launcher returned zero. The test waited for the installation folder and uninstall registration to disappear, then verified the SQLite database hash was unchanged by uninstallation. NSIS's initial uninstaller process hands off to a temporary process; waiting for the launcher alone is insufficient. The test does not select app-data deletion.

Evidence files are generated under ignored `test-results/`: `installer-smoke.json`, `release-smoke.json`, and `installed-release.png`. The installer build is also ignored. Rebuilding may produce a different hash; record new measurements rather than treating this manifest as proof of a later artifact.

## Remaining release checks

Clean-profile onboarding, machines without WebView2/runtime/model, upgrade/rollback behavior, interactive installer pages, signing, account-specific connectors, and the rest of the full acceptance checklist remain unverified or incomplete. The existing profile used here is not evidence that a new user can complete setup without developer assistance.

## Third-party notices (2026-09-09)

- npm production dependencies: 110 packages, all MIT/Apache-2.0/ISC (`docs/THIRD-PARTY-NOTICES.npm.csv`, generated with `license-checker --production`; the single `UNLICENSED` row is the `locallm` workspace itself). `npm audit --omit=dev` reports zero vulnerabilities across 250 total packages.
- Rust dependencies: full normal-dependency `cargo tree` inventory in `docs/THIRD-PARTY-NOTICES.cargo.txt` (557 locked crates). `cargo audit` reports no known vulnerabilities; it lists unmaintained-crate warnings (for example `proc-macro-error`, `unic-*`) that are transitive build dependencies, not audited application code.
- TrueForge catalog/skill presets: MIT, see `catalog/TRUEFORGE-LICENSE` and `catalog/PROVENANCE.md` (upstream `truefoundry/trueforge` commit `e956915`).
- Skill packages: per-package license files are pinned inside `catalog/skills.lock.json` and installed verbatim.
- Runtime/model: pinned llama.cpp b10855 archives and the MiniCPM5-2B Q6_K community quantization are recorded in `docs/RUNTIME.md` and `catalog/runtime-assets.json`; their upstream licenses ship with the downloaded archives, not in this repository.

The production npm dependency audit (`npm audit --omit=dev --json`) reported zero known vulnerabilities on this date. That result is limited to the audited npm dependencies; it is not a complete security audit of the application or native dependencies.
