# Implementation prompt: reliable model downloads, runtime installation, and settings

Work in `C:\Users\Papan Ghosh\Desktop\Projects\LocalLM`.

Implement and verify the repairs described in `docs/SETTINGS-MODELS-RUNTIME-AUDIT-2026-10-05.md`. Read `AGENTS.md`, `DESIGN.md`, and `PRODUCT.md` first. Review the current implementation before changing it, because this checkout contains substantial existing work. Preserve unrelated changes and user data. Follow the current full-window Settings rail and Graphite design conventions. Do not replace the settings interface or introduce an unrelated download framework.

The user's reported failures are concrete:

- While the PrismML runtime is downloading, another model cannot be downloaded.
- A runtime archive appears approximately 97% complete, then the bar appears to restart, with no useful explanation.
- Downloads are unreliable after interruptions, and settings/model/runtime states become confusing or inconsistent.

The audit traced the first two symptoms to a shared installation lock and per-archive progress resets. It also found defects in runtime persistence, multi-file recovery, disk-space accounting, HTTP resume validation, selection rollback, and settings saves. Repair the root causes and add tests for the actual failure paths. Continue through implementation, verification, and any necessary repairs; a cosmetic progress change alone is insufficient.

**First protect the current installation data**

Inspect active jobs and recognizable pending files without changing them. The audit found a completed 312,971,133-byte PrismML archive and a 262,797,957-byte partial CUDA archive in an abandoned `.locallm-runtime-*` directory under `%APPDATA%\app.locallm.desktop\experimental\bonsai-27b\runtimes`. Current contents may have changed since then. Treat existing archives and `.part` files as recovery input; do not clear the app-data directory or redownload everything as a repair strategy.

Do not rebuild or restart the live development app while it is transferring files. The current Playwright web-server setup invokes the worker build and can trigger Tauri's watcher. Establish an isolated test/app-data setup or reuse a suitable frontend server without rebuilding the backend. Never run native smoke tests that write to the user's actual models, selection, or runtime configuration. Preserve interrupted files even if their ownership or integrity cannot be established; surface a reviewable recovery state.

**1. Admit independent acquisition jobs safely — F01/F08**

Inspect `src-tauri/src/lib.rs`, `model_install.rs`, `model_library.rs`, `runtime_install_commands.rs`, and `experimental/bonsai.rs`.

Remove application-wide rejection of unrelated model and runtime transfers. Use a small bounded concurrency limit that allows at least a PrismML runtime and an independent model to download together. Keep duplicate requests for the same destination/immutable asset serialized or deduplicated. A saturated limit should produce an explicit queued job or actionable admission state; never pretend a rejected request is downloading. Loading, selection changes, deletion, experimental opt-out, and extraction/publication must retain the synchronization needed to protect resources they actually share.

Use the smallest coherent job representation that serves the existing acquisition paths. Give jobs stable IDs and asset identity, phase, actual saved/download bytes, expected total, stage/asset progress, terminal result, timestamps, and retry information where relevant. Add a sequence/revision if needed to reject stale responses. Cancellation must name the intended job and never stop another panel's transfer. Define cancellation/pause/resume semantics accurately in both backend and UI.

**2. Preserve downloads across retry, cancellation, and restart — F02/F03**

Inspect `download.rs`, `runtime_install.rs`, `install_recovery.rs`, `model_library.rs`, and startup recovery in `lib.rs`.

Keep archives and partial transfers in stable application-owned storage keyed by immutable revision/hash/size. Use separate disposable staging for extraction, with atomic publication of a complete bundle. Reuse verified archives; revalidate retained bytes before publication. A failure while downloading archive two must preserve archive one and usable bytes of archive two. A cancellation must not discard usable bytes or alter unrelated saved preferences.

Persist recovery metadata atomically and reconcile it on startup. Mark interrupted work accurately; offer a working Resume action. Recover both standard and experimental storage and recognize the current `{filename}.part` convention. Safely adopt recognizable legacy staging/cache layouts after validating pinned identity. Do not follow links/junctions outside owned storage or delete unfamiliar content. Keep blocking extraction workers and staging ownership coordinated until they stop writing.

For Hub models, persist every shard and selected projector, including repository, immutable revision, expected size/hash, destination, and saved bytes. Merge inventory by full identity rather than global filename. Completing shard one must not suppress the pending row's provenance. Keep `download.json` or equivalent recovery metadata until the completed manifest has passed validation and been published. Propagate manifest write failures instead of ignoring them.

**3. Harden transfer recovery and storage preflight — F04/F05/F06**

Implement bounded retry/backoff for transient connection errors, interrupted bodies, timeouts/stalls, rate limiting, and recoverable server responses. Respect `Retry-After`; cancellation must interrupt backoff. Surface retry attempts and eventual failure. Authentication/license errors, missing assets, integrity mismatches, and disk/write errors require appropriately different handling. Keep diagnostics useful while redacting tokens and signed URL query strings.

Validate resumed 206 responses before appending: supported range unit, exact requested start, valid end, pinned total, and consistent body length. Preserve the good prefix on invalid range responses. Handle 200 responses to Range and 416 deliberately; explain unavoidable restarts, and recalculate full-transfer space before discarding a prefix. Keep immutable revisions, HTTPS, pinned size, final SHA-256, and verification-before-publication guarantees. Refresh the original immutable download URL on retries if redirect URLs expire. Check authorized gated-model downloads and cross-host redirect handling without leaking read tokens.

Use one storage calculation in info endpoints and execution. Required space must account for remaining download bytes, reusable cache, extraction bytes, verification overhead actually needed, and reserve. A nearly complete partial must remain resumable when the missing bytes plus reserve fit even though the full original asset no longer fits. Coordinate storage accounting across admitted jobs so each cannot assume it owns the same free capacity. Verification of an existing complete file should not be treated as a new full download.

**4. Connect accurate progress and errors to the whole app — F07/F08/F09**

Inspect `src/lib/api.ts`, `types.ts`, `ModelDownload.tsx`, `RuntimeDownload.tsx`, `ModelLibrary.tsx`, `DownloadActivity.tsx`, `Models.tsx`, and `features/bonsai/BonsaiSettings.tsx`.

Extend the existing ordinary-model activity banner to standard runtime, PrismML runtime, and Bonsai model jobs, including concurrent jobs. Keep progress and cancellation available across Settings sections and Chat. Restore active and interrupted jobs after remount/restart. Avoid duplicated polling sources that can disagree; use one shared subscription or coordinated status source where practical.

Show stable aggregate download bytes, current archive/file name and index, and separate verification/extraction/finalization stages. Switching from archive one to two must not look like restarting the whole transfer. Identify saved bytes versus new network transfer. A fully received archive or complete network total is not an installed/ready bundle until verification and publication succeed. Rate and ETA must not count hashing as network traffic. Use indeterminate progress for phases without measurable work rather than invented percentages.

Update job status independently from disk/catalog metadata. Failure of an advisory info request must not suppress a busy status or cancellation control. Retain useful state and clearly mark it stale when the status request itself fails. Persist terminal error information across navigation and identify the affected asset. Match managed-model cards to their job identity and expected path. Keep existing explicit Use/Select behavior after successful verification.

**5. Make inventory, selection, and settings coherent — F10/F11/F12/F13/F14/F16**

Distinguish downloaded, verified, complete, selected, loading, and running states. Managed inventory must check every required shard/projector against expected metadata and refuse incomplete or altered assets. Preserve a supported local-import flow without falsely claiming downloaded verification. Reuse a complete pinned runtime rather than creating another identical installation; preserve the separation between standard llama.cpp and PrismML. Use the same completeness checks in inventory and automatic runtime discovery, so a broken candidate does not hide a valid one.

Make model switching and combined preference/config changes atomic using the store's existing transaction support where appropriate. Validate before committing and implement explicit recovery if loading fails. Preserve or restore the previous model selection, runtime executable, projector, configuration, profiles, and actual running state. Preserve the failed load log. Refresh the frontend from actual backend state after errors instead of manufacturing a stopped/null-model status. Cover ordinary Use/Load and Bonsai activation/reload consistently.

Await settings saves, prevent duplicate unresolved submissions, and order responses so an older save cannot overwrite a newer draft or display a misleading completion message. Do not reset dirty fields on unrelated bootstrap refresh. Ensure executable changes persist in the model's profile. Make save success, reload success, and refresh success distinct outcomes. Apply the same confirmed/draft and failure-recovery discipline to the optimistic Browser settings save.

Keep runtime status observable while startup is in progress and after the child exits; do not require waiting for the entire load mutex to get status. Report requested/saved configuration separately from effective launch configuration and actual loaded context. Align the older-runtime Automatic GPU fallback with the frontend: unknown cache layouts must not be assumed to consume zero memory. Preserve conservative behavior for unsupported architectures and constrained hardware.

**6. Prove the repaired failure paths — F15**

Repair the relevant smoke scripts' shadowed `page` variables and obsolete navigation selectors. Investigate the existing Bonsai Playwright failure using console, module/network, and fixture evidence. Do not replace assertions with sleeps or broadly increase timeouts to claim success. Ensure all mounted APIs are stubbed, as `AGENTS.md` requires. Keep acquisition tests independent of real GitHub/Hugging Face availability and the user's app-data. Use deterministic small assets with valid hashes and local test servers; production HTTPS requirements must remain enforced.

Add regression tests that fail against the current behavior and pass with the repairs. At minimum cover:

1. PrismML plus an independent model downloading simultaneously; duplicate same-asset requests do not create competing writers; saturation is visible.
2. Two-archive progress remains understandable and aggregate bytes do not reset on transition; verification/extraction is distinguished from downloading.
3. Archive two fails near completion; retry reuses archive one and requests only missing bytes. Cancel and application restart retain usable data and reconstruct an interrupted job.
4. Interrupted second shard and interrupted projector preserve immutable provenance, accurate totals, and Resume actions; identical filenames from different repositories remain distinct.
5. Remaining-byte disk preflight permits resume with limited free space; verification-only and cached-runtime cases work; competing jobs cannot overcommit free space.
6. Valid 206 resume; incorrect/missing range; ignored Range/200; 416; early EOF; corrupt partial/hash; expired redirects; transient 429/5xx; stall; disk failure; cancellation during retry/backoff/extraction. No unverified output is published.
7. Navigate Models → Runtime → Chat → Models during acquisition. All applicable jobs retain their progress, cancellation, and errors. Advisory metadata failure does not hide status.
8. Missing/truncated shard or projector is refused; incomplete runtime is skipped in favor of a complete compatible bundle; already-installed pinned runtime is reused.
9. Failure before spawn and failure after stopping the old runtime leave documented, coherent selection/runtime state and retained diagnostics, for ordinary models and Bonsai.
10. Duplicate or out-of-order saves, dirty drafts during refresh, per-model executable restoration, failed Browser settings save, and effective loaded configuration reporting.
11. Unknown architecture/cache metadata on an older runtime produces conservative GPU allocation, including multiple inference slots.
12. The normal browser-test path cannot rebuild/restart an unrelated live Tauri app or touch its downloads.

Run and complete the repository-required frontend checks:

```text
npm run typecheck
npm run lint
npm test
npx playwright test
```

Also run focused Rust regression tests, the full Rust suite, and the project's relevant Clippy/native acceptance checks using isolated state. If a failure is unrelated, establish and report the baseline with evidence; do not silently skip it or claim a green suite. Existing audit results were 460 frontend tests passing with one skipped, passing targeted Rust tests, and a failing Bonsai browser test. Passing those existing unit tests alone is insufficient acceptance.

Finish with an implementation report mapping F01–F16 to fixes and tests, listing changed files, migration/recovery behavior, verification commands/results, and any demonstrated remaining limitations. Explicitly verify the user's original workflow: PrismML runtime and another model can be acquired reliably, stage transitions make sense, interruption resumes retained bytes, and unrelated saved settings and conversations remain intact.
