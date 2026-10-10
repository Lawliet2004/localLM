# Settings, models, and runtime audit — 5 October 2026

The two reported symptoms have identifiable causes. Unrelated installations are rejected by one application-wide lock. Runtime progress represents the current archive, and resets when the installer advances to the next archive. There are also substantive recovery, persistence, and settings-consistency defects that make an ordinary interruption harder to recover from.

This is an audit and implementation handoff, not a set of applied fixes. Production source was not patched. The checkout already contained extensive uncommitted work; findings refer to the files inspected in that checkout. Existing tests, isolated component probes, and read-only file inspection were used. Static findings are identified separately from observed behavior. The accompanying implementation prompt is `docs/SETTINGS-MODELS-RUNTIME-FIX-PROMPT.md`.

**What was happening with PrismML**

The pinned PrismML bundle contains two archives, in this order:

| Archive | Download size | Purpose |
| --- | ---: | --- |
| `llama-prism-b10754-2459f68-bin-win-cuda-12.4-x64.zip` | 312,971,133 bytes | PrismML executables and backend libraries |
| `cudart-llama-bin-win-cuda-12.4-x64.zip` | 391,443,627 bytes | CUDA runtime libraries |
| Total | 704,414,760 bytes | Both archives are required |

At the first disk inspection, the first archive existed at its complete pinned size, and the second archive's `.part` contained 194,781,940 bytes. Later, that partial file contained 262,797,957 bytes. Thus the first apparent restart was a transition to another required archive. A 97% reading for archive one represents approximately 43% of all download bytes, before extraction and publication.

There was also a later, distinct interruption during this audit. The normal Playwright setup runs `npm run dev`, which rebuilds `src-tauri/resources/web/worker.mjs`. That file was rewritten at 19:04:16 IST, the partial archive stopped advancing around 19:04:17, and the running development app was replaced by a new process starting at 19:05:30. The timing strongly indicates that the test setup triggered Tauri's rebuild/restart watcher. Subsequent browser probes reused the existing server without rebuilding. The complete archive and partial archive were left intact; the current installer does not reconnect that abandoned staging directory. This incident occurred after the originally reported progress reset and should not be confused with its cause.

**Architecture checked**

| Acquisition path | Backend state | Lock held for the complete operation | Persistent activity banner |
| --- | --- | --- | --- |
| Managed MiniCPM model | `model_installer` | `installation_operation` | Yes, ordinary model status |
| Hugging Face models, shards, projectors | `model_installer` | `installation_operation` | Yes, ordinary model status |
| Standard llama.cpp runtime | `runtime_installer` | `installation_operation` | No |
| Bonsai model | `bonsai_installer.model` | `installation_operation` | No |
| PrismML runtime | `bonsai_installer.runtime` | `installation_operation` | No |

`DownloadActivity.tsx` already provides persistent progress, errors, and cancellation for ordinary model downloads. An app-wide absence of download progress would therefore be an incorrect finding. Its coverage stops at `api.modelInstallStatus()`.

The review followed the acquisition commands, shared HTTP downloader, runtime staging/extraction, disk inventory, startup cleanup, experimental activation, model profiles, configuration persistence, runtime launch/status, and the settings components that expose them. General appearance and permission settings were checked for integration context. This does not constitute a complete audit of unrelated connectors, cloud execution, or web retrieval.

**Prioritized findings**

P1 denotes a defect that directly prevents the requested workflow or undermines reliable recovery. P2 denotes a correctness, visibility, or verification defect that should be addressed in the same repair effort.

**F01 · P1 · A runtime download prevents an unrelated model download**

Evidence: `src-tauri/src/model_install.rs:111`, `model_library.rs:294`, `runtime_install_commands.rs:60`, and `experimental/bonsai.rs:302` all call `try_lock()` on the same `installation_operation`, holding it across network transfer and extraction. Failure returns “Another model or runtime installation is running.” There is no accepted queue entry. The UI reads separate installer states, so another panel can still offer an enabled Download button while the backend rejects it.

Trigger: start PrismML runtime installation, then request an ordinary Hugging Face model or Bonsai weights. This explains the reported inability to download another model. Fix with bounded independent transfers and per-destination exclusion, keeping selection/loading synchronization separate. Simply deleting the global lock would allow writers to race on the same `.part` file.

**F02 · P1 · Runtime retries discard downloaded bytes; restarts abandon them**

Evidence: `runtime_install.rs:56` creates a random `TempDir` for every attempt, and archives live inside it at line 72. Error or normal cancellation releases its owner and normally deletes both completed archives and partial downloads. A process termination can instead leave the directory behind. A subsequent attempt creates a different directory and ignores those files. `lib.rs:201` only invokes cleanup for standard `models` and `runtimes`, not experimental storage. `install_recovery.rs:101` recognizes legacy random download filenames but not the current `{archive}.part` naming inside standard runtime staging.

Observed: a PrismML staging directory containing a complete first archive and a substantial second partial survived the development-app restart. There is no persisted job or startup reconciliation to resume it. Separate a stable cache keyed by immutable asset identity from disposable extraction staging, and safely migrate or report recognizable abandoned attempts. Preserve useful bytes on transient failure and cancellation.

The deletion behavior follows both the code and the documented lifetime of [tempfile's TempDir](https://docs.rs/tempfile/3.27.0/tempfile/struct.TempDir.html).

**F03 · P1 · Interrupted shard/projector downloads can lose their resume provenance**

Evidence: `model_library.rs:532` obtains provenance from `model.json`, while pending provenance lives in `download.json`. After the first shard is published, `scan()` creates an incomplete row from final `.gguf` files using the absent completed manifest. At lines 631–634, the pending row is suppressed by an existing row with the same filename or path, rather than merging its repository, revision, expected total, and projector selection into that row. Filename suppression also spans the accumulated inventory, so two repositories with an identical filename can hide one another's pending records.

Trigger: finish shard one, interrupt shard two, then reopen My Models. The row can lose repository/revision and report only discovered final bytes. The same problem occurs when weights finish but their projector does not. `ModelLibrary.tsx:213` requires that provenance for direct resume. The existing scanner test covers a partial first file, not this case. In addition, `model_library.rs:409` removes `download.json` before GGUF validation, cancellation checking, and completed-manifest publication. Failures in that interval remove the most useful recovery metadata.

Fix with one atomic recovery manifest describing every expected file, and merge inventory by immutable model/job identity. Delete pending state only after successful publication of the complete manifest.

**F04 · P1 · Low disk space can incorrectly block a resumable managed model**

Evidence: `model_catalog.rs:69` and `experimental/bonsai.rs:257` advertise full model size plus reserve even when most bytes already exist in `.part`. `ModelDownload.tsx:44` and line 53 use that number to disable Download. In contrast, `download.rs:180` checks remaining bytes plus reserve.

Example: a 1 GiB model has 900 MiB saved and 400 MiB free. The remaining 124 MiB plus 256 MiB reserve fits, but the UI asks for 1,280 MiB and prevents the resume request. An isolated component probe using simulated info metadata confirmed the full-size gate; no real low-disk condition was induced. Return saved/remaining bytes and use the same storage calculation in the UI and backend, including verification-only and cache-reuse cases.

**F05 · P2 · Recoverable transport failures require manual intervention**

Evidence: `download.rs:231` performs one request; `download.rs:263` fails after a 60-second chunk stall. Connection errors, interrupted bodies, 429 responses, and temporary server failures terminate the attempt. Existing partial-model resume is useful, but it only takes effect when another call is made. Runtime retries additionally suffer F02.

Fix with bounded retries and backoff for explicitly recoverable errors, respecting `Retry-After`. Cancellation must interrupt transfer and backoff promptly. Distinguish access/license failures, missing assets, transport failures, integrity failures, and local write errors. Current text such as “Could not connect” loses diagnostic distinctions. Preserve the two-hour safety bound appropriately; do not turn retries into an unbounded loop.

**F06 · P2 · Resume accepts any 206 response without validating the returned range**

Evidence: `download.rs:236` treats status 206 as sufficient to append to the saved prefix. It does not check `Content-Range` start, end, total, or matching body length. An incorrect range is appended; final SHA-256 then rejects it and the error cleanup at lines 304–309 deletes the accumulated partial. The integrity check prevents publication of corrupt output, but good saved bytes can be lost. A 200 response to Range truncates the old prefix at line 239, resetting progress without a specific explanation or a renewed full-transfer space check.

Fix by validating the single-range response before modifying saved bytes, testing malformed/missing ranges and 416 responses, and explicitly handling a server that ignores Range. Immutable revision, size, and SHA-256 remain mandatory; use representation validators where available. [RFC 9110, section 14.4](https://www.rfc-editor.org/rfc/rfc9110.html#section-14.4) defines the range metadata that must be interpreted for safe recombination. This finding is based on source inspection, not a reproduced upstream server error.

**F07 · P1 · The runtime bar restarts without identifying the next archive**

Evidence: `runtime_install.rs:73` emits zero received bytes for each asset, with that asset's size as total. Extraction replaces the counters again at lines 86–90. `RuntimeDownload.tsx:54` displays only “Downloading runtime archive…” and a bar; it has no archive name, index, aggregate byte count, or completed-stage accounting. PrismML uses this same component and installation loop.

Observed: disk contents prove advancement from archive one to archive two. Fix with stable job progress plus separate asset/stage progress. Show “archive 2 of 2” and meaningful downloading, verifying, extracting, and finalizing states. Network bytes reaching their total must not claim that the complete installation is ready. Resumed hashing must not be represented as newly downloaded bytes.

**F08 · P2 · PrismML and runtime activity disappears outside their settings panels**

Evidence: `DownloadActivity.tsx:20` polls only the ordinary `model_installer`. Bonsai and both runtime paths have distinct states. `ModelDownload`, `RuntimeDownload`, and `BonsaiSettings` use independent polling loops tied to mounting. Leaving the applicable page loses their visibility and controls. Install status has no job ID, asset identity, sequence, or timestamps (`model_install.rs:7`, `api.ts:47`), and standard status is shared across managed and Hub models. A managed-model card can consequently display or cancel another ordinary model's job, although its ready-path check correctly prevents selecting another model's result.

Component probes also confirmed that ModelLibrary alone has no Discover progress/cancel display and ignores polled `status.error`. Ordinary models are covered by the outer banner, so these component observations are not an additional app-wide ordinary-model defect. Extend the existing banner and shared subscription/state to every acquisition job, with cancellation bound to the displayed job and terminal errors surviving navigation.

**F09 · P2 · Metadata failure hides independently available install status**

Evidence: `ModelDownload.tsx:27` and `RuntimeDownload.tsx:33` fetch info and status with `Promise.all`, committing neither if either call fails. A failed free-space check or experimental info lookup can therefore suppress a successfully returned busy status and its cancellation control. An isolated model component probe confirmed this behavior. Conversely, a panel with a stale busy status can retain it indefinitely while info fails.

Fix by updating job status independently of advisory metadata, keeping errors attributable to each request. Settled requests must not override newer job/action state. An info failure must not hide cancellation or make an active job appear idle.

**F10 · P2 · Inventory can label altered or incomplete model files as complete**

Evidence: `model_library.rs:555` computes completeness from shard names plus existence of a deserializable `model.json` in a managed directory. The manifest has no per-file expected sizes/hashes and scanner completeness does not check declared projector existence. Discovered current sizes become the expected-looking total. Non-managed models are considered complete from naming alone. Runtime loading validates a GGUF header, not all expected tensor/shard bytes.

Trigger: truncate a published managed shard while retaining its GGUF header, or remove a manifest's projector. The library can still offer Use. This is distinct from the valid SHA-256 verification already performed during transfer. Store expected file metadata and a clear verification state; check every required file at inventory and use boundaries. Preserve a separate local-import path so arbitrary user GGUFs are not falsely described as downloaded and verified.

**F11 · P1 · Ordinary model switching commits settings before knowing whether loading succeeds**

Evidence: `model_library.rs:1028` saves new preferences, then runtime configuration, then a profile, before `load_selected_model()`. There is no ordinary-model rollback. The store already has a transaction for combined selection/configuration (`store.rs:2153`), but this path does not use it. Missing DLLs, invalid context, or failed startup can leave the new model selected even when the previous process remains active after an early validation failure, or no process remains after a later failure. `App.tsx:902` manufactures a null-model error status for any rejected load instead of reconciling actual runtime state. Bonsai activation already implements a separate previous-selection recovery path.

Fix by validating before committing, using an atomic combined update, and defining rollback/reload behavior. After failure, refresh actual persisted preferences and actual runtime status. Preserve the original failure diagnostics. Apply coherent behavior to Use, Load, Apply configuration, and experimental activation.

**F12 · P2 · Saves can race, and saved profiles can lag behind settings**

Evidence: `RuntimeForm.tsx:67` invokes a callback typed as returning void; the form has no pending-save state. `Models.tsx:161` likewise lacks save-in-progress exclusion. An isolated probe confirmed two unresolved runtime saves can be submitted. The backend save commands (`commands.rs:235`, line 253) do not participate in the model-operation lock. Database state, response-budget fitting, profile writing, and frontend refresh are separate stages. A late first refresh can reset the newer draft (`Models.tsx:73`, `RuntimeForm.tsx:28`). `save_preferences` does not persist the active runtime profile, whereas saving configuration does, so switching away and back can restore an older executable selection.

Fix with explicit pending-save UI state, awaited callbacks, ordered/revision-aware responses, and coherent persistence. Do not overwrite dirty drafts on unrelated refresh. Make a successful save mean all required state was saved; distinguish a successful save followed by a failed reload or refresh.

**F13 · P2 · Runtime discovery and reported runtime state are inconsistent**

Evidence: `commands.rs:293` chooses a standard runtime from executable presence alone, even though `runtime_inventory.rs` already checks the pinned file set. It can select an incomplete candidate and fail while another complete bundle exists. `BonsaiSettings.tsx:26` supplies an empty runtime list, so experimental installations do not share inventory/reuse controls. Repeated installation always creates another random directory. `commands.rs:290` holds the runtime mutex throughout loading; `runtime_status` needs that same mutex at line 362. `App.tsx` does not independently subscribe to runtime process changes, so externally exited processes and intermediate load states are not promptly reflected.

Fix by reusing complete pinned bundles through one consistent inventory, retaining standard/Prism compatibility boundaries and user selections. Publish/read operation status without holding up status reads behind a whole startup. Reconcile process exit and report saved/requested settings separately from effective loaded context/configuration (`runtime.rs:271` and line 279).

**F14 · P2 · Older-runtime GPU fallback treats unknown cache size as zero**

Evidence: `runtime.rs:347` uses an internal layer estimator when Automatic is selected and the executable lacks `--fit`. `runtime_config.rs:149` substitutes zero for an unknown KV-cache estimate. In contrast, `runtimeGuidance.ts:48` recommends CPU when the memory layout is incomplete. Unsupported/hybrid architectures can therefore receive more GPU layers in the backend fallback than the settings explanation promises, risking allocation failure. This path is conditional on an older runtime lacking `--fit`; no claim is made that PrismML currently follows it.

Fix by aligning frontend and backend conservative behavior for unknown metadata and validating the effective launch configuration. Add focused tests for unknown architectures, missing cache dimensions, multiple slots, and constrained free VRAM.

**F15 · P2 · The verification harness misses key acquisition scenarios and can restart a live app**

Evidence: `RuntimeDownload.test.tsx` covers one successful selection flow. The runtime installer test covers cancellation before any transfer. Bonsai e2e covers opt-in, with mocked idle acquisition state. These do not cover archive two failure, resume after restart, competing downloads, or state restoration across pages. Several native smoke files declare a new uninitialized `let page` inside `try`, shadowing the connected page: `model-download-info-smoke.mjs:6`, `model-cancel-smoke.mjs:7`, `runtime-inventory-smoke.mjs:6`, and `runtime-diagnostics-smoke.mjs:6`. Other model/runtime smoke scripts still click the obsolete “Models & runtime” navigation instead of the current Settings rail.

Observed: the permission e2e passed, while Bonsai e2e failed before locating its toggle in the normal run. Isolated runs without a worker rebuild also failed, once at reload and once while Settings was still opening. The root cause of that e2e instability was not established; these failures are not proof of a particular backend download defect. Increasing the overall test timeout did not consistently resolve them. The debug CDP endpoint on port 9223 was unavailable, so direct native command-state inspection could not be completed.

Fix and investigate fixtures, imports, selectors, and timing using console/network evidence; do not mask failures with sleeps. Provide isolated deterministic network/storage fixtures and a non-rebuilding browser-test path. Make worker generation avoid rewriting identical output, or otherwise keep tests from triggering a live Tauri watcher. Native acquisition tests must use disposable app-data and must not share user downloads.

**F16 · P2 · Browser settings can display an unpersisted selection after a save failure**

Evidence: `Settings.tsx:175` changes local config before awaiting `saveBrowserConfig`, without rollback or pending-save exclusion. On rejection, the UI retains the requested value and shows an error; reopening can restore the older persisted value. Overlapping requests can also leave completion messages out of order. This is an additional settings persistence issue, not a cause of the PrismML transfer symptoms.

Fix with confirmed versus draft state, ordered writes, and accurate failure/rollback behavior. Keep the existing immediate interaction pattern if desired, but ensure the UI accurately identifies unsaved state.

**Verification recorded**

| Check | Result |
| --- | --- |
| `npm run typecheck` | Passed: application and e2e TypeScript |
| `npm run lint` | Passed |
| `npm test` | 460 passed, 1 skipped; 80 test files passed, 1 skipped |
| Targeted model/runtime/Bonsai UI tests | 41 passed across 7 files |
| Five isolated component probes | Confirmed full-size resume gate, component-only missing activity/failure display, metadata/status coupling, and duplicate unresolved saves |
| `cargo test --lib download::tests` | 8 passed |
| `cargo test --lib model_library::tests` | 14 passed |
| `cargo test --lib experimental::bonsai::tests` | 7 passed |
| `cargo test --lib runtime` | 31 passed, 2 ignored; includes overlapping runtime-related tests |
| Selected Playwright e2e | Permission test passed; Bonsai test failed; no complete e2e pass claimed |
| Native CDP state read | Unavailable: connection refused at `127.0.0.1:9223` |

The temporary probe tests and temporary no-build configurations were removed after use. Browser failure artifacts remain under `test-results`. The ordinary verification commands do not currently demonstrate end-to-end reliable runtime recovery. No production fix, migration, real multi-gigabyte failure injection, or full native installer acceptance run was performed.

**Implementation order**

1. Protect existing partials and document recovery/migration before running any destructive cleanup. Address F02/F03, then introduce job identity and safe independent-transfer admission for F01/F08.
2. Repair retry, range validation, and storage accounting: F04–F06. Preserve pinned hashes and verification before publication.
3. Wire accurate job/stage progress and navigation recovery: F07–F09. Extend the existing activity banner.
4. Correct inventory and selection/configuration consistency: F10–F14 and F16.
5. Repair acquisition integration coverage and isolated test startup: F15. Add a regression scenario for each repaired defect and run the repository-required checks.

The most valuable acceptance scenario is the user's actual workflow: start PrismML, start another model, navigate between Settings and Chat, interrupt a transfer near completion, reopen the app, resume only missing bytes, and end with two verifiably complete assets and unchanged unrelated model settings.
