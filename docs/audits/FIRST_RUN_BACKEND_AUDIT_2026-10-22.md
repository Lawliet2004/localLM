# LocalLM backend and first-run audit — 22 October 2026

The current code passes its automated gates, but first-run readiness is not established. The most serious issues found are destructive download recovery, memory recommendations that can exceed the machine's RAM, and synchronous credential-store operations while holding the shared database mutex.

This report records **25 findings: 3 high priority, 21 medium priority, and 1 low priority**. Some medium findings are product/setup gaps rather than implementation defects; each is identified. Seven specific observations were reproduced in isolated fixtures. The remaining findings are supported by source traces, with platform acceptance explicitly separated below.

No application fixes were made. Existing uncommitted changes were preserved. Only this report and its isolated reproduction files were added.

## Scope and limits

- Audited the current working tree at base commit `a7d1321`, including the user's existing uncommitted frontend/prompt changes.
- Traced startup, runtime provisioning and discovery, Hugging Face discovery/download, disk accounting, resume/recovery, library enumeration/deletion, hardware telemetry, memory recommendations, runtime launch, provider credential access, browser dependencies, and release configuration.
- Used the full Rust/frontend suites for broader backend contract coverage. This is a focused source-assisted audit, not a line-by-line proof of every backend module or a promise that every possible fault has been found.
- Did not launch a browser preview, operate the user's desktop, modify credentials, install dependencies, install/uninstall the app, download full models, or run paid provider requests.
- Linux host verification does not establish Windows/macOS installer, driver, GPU or OS-keychain acceptance. Historical release documents are treated as historical evidence.

Priority: **P1** = substantial loss of download work, unsafe recommended resource demand, or cross-feature blocking. **P2** = broken recovery, misleading diagnostics, compatibility/setup failure, or material release gap. **P3** = narrow correctness risk with a less common trigger.

Evidence labels: **Reproduced** means an isolated execution demonstrated the observation; **Source** means the current code directly implements the described behavior; **Gap** means missing supported setup/acceptance rather than a claim that all affected machines crash.

## Verification performed

| Check | Observed result |
|---|---|
| `cargo test --lib` | **981 passed, 0 failed, 19 ignored**; 1,000 total |
| `cargo check --lib` | Passed |
| `cargo clippy --lib --all-targets -- -D warnings` | Passed |
| `npx vitest run` | **845 passed, 1 skipped**; 117 passing test files, 1 skipped |
| `npm run typecheck` | Passed |
| `npm audit --omit=dev --json` | 0 reported vulnerabilities in the installed npm dependency graph |
| Isolated Rust source fixture | Demonstrated five recovery/inventory observations, detailed below |
| Execution of transpiled current TS guidance functions | Demonstrated oversized recommendation and first-GPU-only fit classification |
| HEAD requests to pinned managed-model and two runtime URLs | All returned HTTP 200 with expected Content-Length |

The frontend suite emitted a jsdom canvas `getContext()` warning but passed. The Rust connector malformed-protocol fixture took over 60 seconds and completed successfully.

The current run exercised the non-ignored local socket fixtures successfully. The repository documents socket restrictions in earlier sandboxed sessions; those earlier restrictions do not imply that today's executed tests were skipped. The 19 ignored tests remain unexecuted, including the live Hugging Face transfer, cached CUDA archive/runtime checks, browser/desktop integrations, external debugger/language-server/profiler tests, remote database fixtures, and explicitly opt-in transport/chat cases.

No current RustSec audit was run: `cargo-audit` was not installed. No production installer or release bundle was built. An npm advisory result is not a native dependency or application security clearance.

Pinned asset availability observed in this audit:

| Asset | HEAD result | Bytes |
|---|---|---:|
| `MiniCPM5-2B.Q6_K.gguf` at the pinned revision | 200 | 2,070,227,904 |
| `llama-b10855-bin-win-cuda-12.4-x64.zip` | 200 | 254,069,159 |
| `cudart-llama-bin-win-cuda-12.4-x64.zip` | 200 | 391,443,627 |

Runtime archives total **645,512,786 bytes**; manifest-listed expanded files total **1,168,007,453 bytes**. The installer reserves another 268,435,456 bytes. The managed runtime therefore asks for about **1.94 GiB free** during installation. Together with the pinned model, final retained weights/runtime files are about **3.02 GiB**, before conversations, artifacts and other caches. Header success establishes availability only; it does not verify complete transfer, checksum, extraction, execution, or future availability.

## High-priority findings

### F01 — Restart deletes resumable model downloads — P1, Reproduced

**Scenario:** Download several gigabytes, cancel or lose power, then reopen LocalLM and choose Resume.

The downloader intentionally preserves `<filename>.part`, but startup calls `install_recovery::clean(..., false)`, which deletes those same files. The user must download that shard/file again. Completed shards survive; the active partial shard does not. This also affects an ordinary quit after cancellation, not only a crash.

**Evidence:** [startup cleanup](/home/papan/Desktop/Projects/localLM/src-tauri/src/lib.rs:441), [partial deletion](/home/papan/Desktop/Projects/localLM/src-tauri/src/install_recovery.rs:60), [resume implementation](/home/papan/Desktop/Projects/localLM/src-tauri/src/download.rs:260).

**Fixture:** cleanup returned `1`; the saved prefix disappeared. Preserve valid resume state across restart, and give users explicit cleanup instead of treating all partial downloads as garbage.

### F02 — Recommended context can exceed available RAM by many gigabytes — P1, Reproduced

**Scenario:** A user clicks Use recommended settings on a small-memory machine with a long-context model.

`recommendedConfig()` chooses the model's maximum context, up to 2,097,152 tokens, without using available RAM to size that context. If GPU fitting fails, it returns CPU mode with the same large context. Memory meters can warn, but the primary recommendation itself remains oversized and saving is allowed.

**Fixture using current TS functions:** 8 GiB total RAM, 4 GiB free, no GPU, a 4 GiB standard-attention model with 131,072 context: recommendation = **131,072 tokens / CPU**; its own estimator reports **8.5 GiB KV cache + 4 GiB weights = 12.5 GiB RAM**, excluding compute/OS overhead. The calculation is reproduced; actual OS OOM or swapping was not induced.

**Evidence:** [recommendation](/home/papan/Desktop/Projects/localLM/src/lib/runtimeGuidance.ts:60), [apply recommendation](/home/papan/Desktop/Projects/localLM/src/components/RuntimeForm.tsx:187). Recommend a memory-bounded context, and preserve “unknown” when the necessary telemetry/layout is unavailable.

### F03 — Provider credential operations still block while holding the shared DB mutex — P1, Source

**Scenario:** First API-key save or provider use on Linux while Secret Service is locked, slow or unavailable.

`save_provider` calls synchronous `save_connection(&*state.database()?, ...)`; that function calls `vault.save()`. Provider deletion similarly calls `vault.clear()`, and ordinary key resolution calls `vault.load()`. These can enter the OS keyring synchronously. In save/delete paths, the shared SQLite mutex remains held during that operation, so unrelated DB-backed features can wait for the keyring. Existing asynchronous vault wrappers do not cover these paths.

**Evidence:** [save command](/home/papan/Desktop/Projects/localLM/src-tauri/src/commands.rs:74), [key resolution](/home/papan/Desktop/Projects/localLM/src-tauri/src/commands.rs:111), [synchronous save](/home/papan/Desktop/Projects/localLM/src-tauri/src/providers.rs:222), [production keyring access](/home/papan/Desktop/Projects/localLM/src-tauri/src/vault.rs:46).

The older backend audit's claim that provider keyring round trips were all moved off async workers is too broad for this working tree. No real keyring was locked or changed during this audit. Move the blocking OS operation outside the DB guard and bound/recover its failure explicitly.

## Download, recovery and storage findings

### F04 — A recovered incomplete entry can be impossible to delete — P2, Reproduced

After F01 removes the partial, `scan()` reconstructs an incomplete entry from `download.json`, with `files` pointing to the now-missing `.part`. Delete calls `remove_file()` and returns immediately on `NotFound`, before metadata cleanup. The entry reappears on refresh; saved progress can also be stale.

**Evidence:** [pending entry construction](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:648), [delete loop](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:778). The fixture returned `NotFound` when deleting the advertised file. Treat already-missing files as removed, then clean their journal/profile.

### F05 — Resume source is lost once the first weight file exists — P2, Reproduced

**Scenario:** First shard completes and the next shard fails, or all text weights finish and the projector transfer fails.

The GGUF scan adds a model row before processing `download.json`. That row reads source/revision/projector from `model.json`, which does not exist until the entire operation succeeds. The later pending-journal branch suppresses a second row. For a generic Hub model, the visible incomplete entry consequently has no repo/revision/projector, even though the journal has them. Resume asks the user to re-enter the repository.

**Evidence:** [manifest-only source](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:554), [row source](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:632), [pending suppression](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:655), [resume UI](/home/papan/Desktop/Projects/localLM/src/components/ModelLibrary.tsx:228). The fixture produced `repo:null`, `revision:null`, and `projectorFilename:null` with an existing journal. Merge pending metadata into the existing row.

### F06 — Nested repository paths are stripped from resume identities — P2, Source

**Scenario:** Download `Q4_K_M/model.gguf`, interrupt before publication, then Resume from My Models.

Inventory reduces `DownloadState.filename` to the basename. The UI passes that basename back to a backend that searches for the exact repository-relative filename. It cannot find `model.gguf` when the catalog contains only `Q4_K_M/model.gguf`. Completed model rows also expose basenames, making re-resolve flows ambiguous.

**Evidence:** [basename conversion](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:667), [exact selection](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:285), [resume call](/home/papan/Desktop/Projects/localLM/src/components/ModelLibrary.tsx:229). Preserve remote filename separately from the local display name.

### F07 — Incomplete downloads with the same basename can disappear — P2, Source

**Scenario:** Two repositories or revisions both contain `model-Q4_K_M.gguf`; one is interrupted.

The pending-entry check compares its basename against the entire accumulated inventory, even across different digest directories. A previously enumerated model with that filename suppresses the pending entry. The interruption can become invisible, preventing ordinary resume/deletion through the library.

**Evidence:** [global basename comparison](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:655). Match directory/source identity, not filename alone.

### F08 — Crash cleanup does not recognize actual runtime partial archive names — P2, Reproduced

**Scenario:** The app is forcibly terminated while downloading a runtime ZIP.

Runtime staging contains `<archive-name>.zip.part`. Cleanup accepts the final archive name or a random `.locallm-download-<32 chars>.part`; it rejects the downloader's actual `<archive>.part`. The staging directory remains and an “Unexpected runtime staging content” error is recorded. Normal cancellation still benefits from `TempDir` cleanup; the defect concerns abandoned crash staging.

**Evidence:** [archive destination](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime_install.rs:73), [accepted names](/home/papan/Desktop/Projects/localLM/src-tauri/src/install_recovery.rs:118). The fixture reproduced the rejection with an otherwise recognized staging name. Recognize each pinned archive's partial name.

### F09 — Deleting a downloaded vision model leaves its projector on disk — P2, Source

Weights and projector share a managed directory, but `InstalledModel.files` deliberately contains weights only. Delete iterates `files`; cleanup removes manifests and `.part` files, but never the separately recorded `projector_path`. The remaining projector keeps the directory nonempty. Large space is not reclaimed, and library-reported model size excludes it.

**Evidence:** [projector excluded from files](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:424), [deletion](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:778), [cleanup](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:795). Delete managed companions with explicit ownership checks and show the full deletion size.

### F10 — Recovery metadata writes are neither atomic nor error-reporting — P2, Source

**Scenario:** Disk fills, writes are denied, or the app crashes while updating `download.json`.

`write_download_state()` uses a direct `std::fs::write` and ignores the result. Readers ignore invalid JSON. A failed/torn journal can silently remove source identity and recovery UI, even when partial data exists. This compounds F01/F05/F07 but has an independent trigger.

**Evidence:** [journal write](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:511), [journal read](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:683). Use atomic journal replacement and surface failures before promising resumability.

### F11 — One broken symlink can fail the entire model inventory — P2, Reproduced

`scan()` canonicalizes every directory entry before checking `is_symlink()`. A dangling link therefore returns an error and aborts enumeration rather than being skipped. A library refresh can fail for all models because of one unrelated entry.

**Evidence:** [enumeration order](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:535). The Unix fixture produced `No such file or directory` from the whole scan. Inspect entry type first and isolate individual-entry failures.

### F12 — Managed downloads are tied to the app-data volume — P2, Gap

**Scenario:** System/profile drive is nearly full, but a second drive has enough space for models.

Both managed model installers use `<app-data>/models`; runtimes use `<app-data>/runtimes`. The download flow offers no alternate managed storage root. Users must move/download files externally and use manual paths, losing a straightforward managed setup. Disk checks themselves are present and do reject inadequate space.

**Evidence:** [Hub destination](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:344), [runtime root](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime_install_commands.rs:99). Offer a persistent managed storage location and migration/recovery handling.

### F13 — Reinstalling runtimes accumulates retained bundles — P2, Gap

Every successful runtime install publishes a fresh UUID directory. Inventory offers selection, while the current command surface provides no managed runtime deletion/pruning flow. Repeated setup/retry after success consumes roughly 1.09 GiB per expanded bundle, excluding any external files. Active/incomplete variants also require manual interpretation.

**Evidence:** [new destination per install](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime_install.rs:113), [installed runtime controls](/home/papan/Desktop/Projects/localLM/src/components/RuntimeDownload.tsx:57). Provide reuse/repair/remove behavior with protection for a loaded runtime.

### F14 — “Never replace an existing destination” is not guaranteed on fallback filesystems — P3, Source

If `hard_link()` fails, `install_verified()` checks existence and calls ordinary `rename()`. On Unix, a destination created between that check and rename can be replaced. The normal hard-link path is atomic; this affects filesystems or errors that force fallback and a concurrent destination creator.

**Evidence:** [fallback](/home/papan/Desktop/Projects/localLM/src-tauri/src/download.rs:445). Existing tests cover the normal path and a destination that already exists, not this fallback race. Use a no-replace primitive or fail explicitly when it cannot be guaranteed.

## Hardware, model choice and runtime findings

### F15 — RAM telemetry is absent on Linux and macOS — P2, Source

`memory()` implements only the Windows API and returns `(None, None)` elsewhere. CPU-only Linux/macOS users see unknown RAM fitness; Linux NVIDIA users can see GPU data but no RAM spill capacity. The app cannot substantiate “fits in system RAM” recommendations on those platforms.

**Evidence:** [memory implementation](/home/papan/Desktop/Projects/localLM/src-tauri/src/hardware.rs:132), [discovery fit calculation](/home/papan/Desktop/Projects/localLM/src/lib/modelStudio.ts:89). Implement platform RAM telemetry and preserve uncertainty when unavailable.

### F16 — Recommendation can disable a usable non-NVIDIA GPU — P2, Source

Telemetry is `nvidia-smi` only. `fittingGpuLayers()` returns `0` for unknown GPU memory or an unsupported cache-layout formula; `recommendedConfig()` turns that into explicit `gpuLayers:0` / CPU mode. Clicking the recommendation can therefore disable Metal/Vulkan/ROCm acceleration even when a manually selected native runtime could use it. For an older runtime without `--fit`, the backend automatic fallback also maps absent telemetry to CPU.

**Evidence:** [NVIDIA query](/home/papan/Desktop/Projects/localLM/src-tauri/src/hardware.rs:89), [unknown becomes zero](/home/papan/Desktop/Projects/localLM/src/lib/runtimeGuidance.ts:47), [CPU recommendation](/home/papan/Desktop/Projects/localLM/src/lib/runtimeGuidance.ts:60), [backend fallback](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime.rs:425). Distinguish unknown capability from confirmed inability; query the selected runtime's devices/backends.

### F17 — Fit classification uses only GPU 0 and a stale snapshot — P2, Reproduced/Source

**Scenario:** Small display GPU plus large compute GPU; or other GPU applications open after Discovery loads.

Discovery and guidance use `gpus[0]`; the discovery page samples hardware at mount and does not continuously refresh it. The selected runtime can choose devices differently. Multi-device capacity and subsequent memory pressure are not represented.

**Fixture:** a 4 GiB model with a 2 GiB first GPU and a 24 GiB second GPU was labeled `large` / Memory tight. This reproduces classification only, not execution on a multi-GPU host.

**Evidence:** [first GPU](/home/papan/Desktop/Projects/localLM/src/lib/modelStudio.ts:95), [single sample](/home/papan/Desktop/Projects/localLM/src/components/ModelLibrary.tsx:60), [backend first GPU](/home/papan/Desktop/Projects/localLM/src-tauri/src/hardware.rs:27). Match guidance to the runtime's device selection and expose a refreshed sample time.

### F18 — Managed runtime provisioning does not match the full hardware/platform range — P2, Gap

The one managed runtime is **Windows x64 CUDA 12.4**. `managed_runtime_supported()` checks only `cfg!(windows)`, not CPU architecture, driver/backend suitability or an alternative CPU/Vulkan build. Linux/macOS users get a message to supply their own native executable; no managed installer is available. An ARM Windows build would also pass the support check and receive x64 assets.

The CUDA bundle includes CPU backend DLLs, so this finding does **not** claim that every Windows machine without NVIDIA fails. It establishes the absence of hardware-aware package selection and a first-run native setup path for other platforms.

**Evidence:** [support check](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime_install.rs:11), [pinned assets](/home/papan/Desktop/Projects/localLM/catalog/runtime-assets.json:1), [unsupported-platform UI](/home/papan/Desktop/Projects/localLM/src/components/RuntimeDownload.tsx:46). Establish supported OS/architecture/backend combinations before offering a runtime download.

### F19 — Model downloads are not preflighted against runtime architecture support — P2, Gap

Discovery validates Hub metadata and hashes and ranks quantizations, but does not check whether the installed runtime supports the model architecture/quantization or a selected projector combination. A valid GGUF header is not evidence that llama.cpp can run its architecture. A user can finish a large verified download and discover incompatibility only at load time.

**Evidence:** [download completion checks](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:434), [runtime capability inspection](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime.rs:405), [projector selection](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:70). Surface compatibility as known/unknown before transfer; distinguish integrity from loadability. No particular live Hub model was deliberately downloaded to force failure.

### F20 — Loading a model has no cancellation path and blocks runtime-state access — P2, Source

`load_selected_model()` holds the runtime mutex throughout launch and health polling. Load holds the exclusive operation lease; unload cannot acquire it, bootstrap/runtime-status wait for the runtime mutex, and `cancel_generation` signals a channel the loader never reads. The UI disables load/unload while busy and offers no load-specific cancel.

Nominal sleep-only polling budgets are **180 seconds** for a first file under 4 GiB and **600 seconds** otherwise; request waits can extend elapsed time substantially. The user may need to quit the application to interrupt a slow first load. Downloads have cancellation; model startup does not.

**Evidence:** [lock across load](/home/papan/Desktop/Projects/localLM/src-tauri/src/commands.rs:399), [poll loop](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime.rs:255), [attempt budgets](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime.rs:457), [generation-only cancel](/home/papan/Desktop/Projects/localLM/src-tauri/src/chat.rs:3385). Add load cancellation and independently readable progress/state.

### F21 — Split models receive a startup budget based only on the first shard — P2, Source

Launch gets `model_bytes` from `metadata(first_shard).len()`, then chooses the startup budget. A 20 GiB model in ten 2 GiB shards receives the under-4-GiB budget. GGUF metadata code already knows how to sum shards, but this decision does not use it.

**Evidence:** [first-file size](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime.rs:171), [budget threshold](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime.rs:457), [existing total-shard calculation](/home/papan/Desktop/Projects/localLM/src-tauri/src/gguf.rs:109). Use total weight size and actual progress/cancellation. A slow physical model load was not forced here.

### F22 — A rejected reload can make the frontend forget a still-running model — P2, Source

Runtime load validates paths/model/context/runtime flags before stopping the old working model. If that preflight rejects the replacement, `App.modelAction()` nevertheless sets frontend runtime state to `phase:error`, `modelPath:null`. The old backend model can still be running and consuming resources while the UI offers Load instead of Unload. This catch does not refresh authoritative backend state.

**Evidence:** [validation before stop](/home/papan/Desktop/Projects/localLM/src-tauri/src/runtime.rs:145), [blanket error state](/home/papan/Desktop/Projects/localLM/src/App.tsx:935), [load/unload control](/home/papan/Desktop/Projects/localLM/src/components/Models.tsx:143). Refresh runtime status after failed loads and retain the actual loaded configuration.

### F23 — Per-model settings collide for paths differing only by case — P2, Source

`profile_path_for()` hashes `path.to_lowercase()` on every OS. On a case-sensitive filesystem, `/models/A.gguf` and `/models/a.gguf` are different files but share a profile. Settings can overwrite each other; deleting one can remove the other's saved profile. The profile includes runtime and projector paths, so this can become a load failure rather than only a changed slider.

**Evidence:** [profile identity](/home/papan/Desktop/Projects/localLM/src-tauri/src/model_library.rs:789). Use platform-appropriate canonical file identity.

## Dependency/diagnostic findings

### F24 — Browser diagnostics can report availability without a browser — P2, Source

`automation_diagnostics()` verifies the pinned package entry and whether the Node path is a file, then reports “Browser engine found.” It does not resolve/launch the selected Chrome/Edge channel or validate the installed browser's ability to start. Setup installs the MCP package with scripts disabled; it does not install Chrome/Edge. A clean Linux machine without Chrome can appear ready until its first browser tool call.

**Evidence:** [diagnostic](/home/papan/Desktop/Projects/localLM/src-tauri/src/automation.rs:668), [package-only installation](/home/papan/Desktop/Projects/localLM/src-tauri/src/automation.rs:692), [documented external browser prerequisite](/home/papan/Desktop/Projects/localLM/docs/AUTOMATION.md:9). Report package, Node and browser readiness separately and test the actual selected executable when the user requests diagnostics.

### F25 — Tool readiness checks do not enforce required Node versions — P2, Source

Web/weather readiness checks whether the Node path is executable, not whether it meets Node 22.13+. Browser setup checks file presence and asks for Node 18+, also without a version probe. An old or wrong executable can pass a prerequisite check and then fail inside the bundled worker/package. Interpreter autodetection searches PATH and does not guarantee a graphical app inherits the developer shell's version-manager PATH.

**Evidence:** [web readiness](/home/papan/Desktop/Projects/localLM/src-tauri/src/web_search.rs:98), [browser setup prerequisite](/home/papan/Desktop/Projects/localLM/src-tauri/src/automation.rs:699), [interpreter detection](/home/papan/Desktop/Projects/localLM/src-tauri/src/execution.rs:74). Probe version and executable behavior; expose feature-specific requirements. The app does give manual path configuration and useful missing-Node errors; the gap is false readiness for an unsuitable existing executable.

## First-time user scenario matrix

These rows distinguish code-supported failure handling, identified issues, and acceptance still required. A source trace is not a passed end-to-end test.

| Scenario | Expected current outcome / audit disposition |
|---|---|
| Fresh Windows x64 + supported NVIDIA + adequate disk/RAM | Managed model/runtime path exists; current clean-install-to-first-answer acceptance not performed |
| Fresh Windows CPU-only / AMD / Intel GPU | One CUDA package offered; CPU DLLs exist, alternative acceleration/package selection absent; F18 |
| Windows ARM64 target | Support check is OS-only, package is x64; native acceptance absent; F18 |
| Fresh Linux NVIDIA | Models downloadable; user must obtain/select native llama-server; no RAM telemetry; F15/F18 |
| Fresh Linux AMD/Intel | Native executable must be supplied; recommendations can force CPU; F16/F18 |
| Apple Silicon / Intel macOS | Native executable must be supplied; RAM/GPU guidance incomplete; native permission/install acceptance absent |
| Small RAM / very long-context model | Recommended context can exceed memory; F02 |
| Unsupported/hybrid cache architecture | Estimate marked incomplete, but recommendation can become CPU; F16 |
| Multiple GPUs | GPU 0 drives app estimates, selected runtime may differ; F17 |
| Other applications consume VRAM after page opens | Fit badge uses old snapshot; F17 |
| Weights fit but KV/image buffers do not | Discovery warns its candidate excludes these; no guaranteed fit or load admission check |
| System disk full, second disk free | Managed destination cannot be chosen; F12 |
| Disk fills mid-transfer | Write failure is returned; resume journal failure can be silent; F10 |
| Cancel, then resume without quitting | Downloader supports byte resume/checksum; full UI transfer acceptance not performed |
| Cancel/crash, reopen, resume | Active partial deleted; F01 |
| Reopen and delete interrupted item | Can fail on nonexistent partial; F04 |
| Completed shard + incomplete next shard/projector | Source/resume metadata lost in displayed row; F05 |
| Nested Hub file path | Basename-only resume fails exact catalog lookup; F06 |
| Two repos/revisions with identical filenames | Pending item can be hidden; F07 |
| Runtime crash mid-ZIP | Unrecognized `.zip.part` can leave staging behind; F08 |
| Repeated successful runtime installs | Retained copies accumulate; F13 |
| Delete multimodal download | Projector remains; F09 |
| Missing/corrupt model or mismatched saved context | Prelaunch errors exist; frontend can misreport an old loaded model; F22 |
| New GGUF architecture unsupported by runtime | Integrity can pass; loadability not preflighted; F19 |
| Slow HDD/network drive/large split model | Long startup cannot be cancelled; first-shard budget may be too short; F20/F21 |
| Gated/private Hub repository | Token input and 401/403/license guidance exist; account-specific acceptance not performed |
| Hub outage, throttling, expired CDN redirect | Transfer retries/ranges exist; metadata lookup can fail separately; no live outage forced |
| Captive portal / TLS interception / proxy authentication | Download fails through bounded requests; generic connection/HTTP messages; enterprise network acceptance required |
| Offline first install | No bundled weights/runtime; requires preprovisioned files or later connectivity |
| Corrupt transfer / wrong Content-Range | Hash/size/range rejection paths have automated coverage; full-size real transfer not exercised |
| Existing corrupt final model file | Verification refuses overwrite; user must remove/repair it; intended protection |
| Broken link in managed library | Entire inventory can fail; F11 |
| Case-sensitive model paths | Settings collide; F23 |
| Missing Node | Web/weather unavailable with configuration guidance; no managed Node installer |
| Old Node or desktop PATH lacks version-manager install | Readiness/version detection incomplete; F25 |
| Research selected, no SearXNG server | Default endpoint is `http://127.0.0.1:8080`; user must configure a service/provider separately |
| Browser MCP installed, Chrome/Edge absent | Diagnostic may look ready; first action fails; F24 |
| Python absent / project packages absent | Manual interpreter/venv/dependency setup required; no universal package environment bundled |
| LSP, debugger, profiler, Git/container tools absent | External prerequisites remain feature-specific; corresponding opt-in acceptance tests are ignored |
| Linux Secret Service locked/unavailable | Credential actions fail or wait; provider DB blocking risk F03; local chat should not require vault unlock just to list history |
| New machine/account restores only SQLite and sealed credentials | OS encryption key may be unavailable; sealed credentials cannot be recovered without it; deliberate vault boundary |
| No project folder chosen | New chat/send is blocked by an explicit project-folder requirement, even after model setup; product behavior to include in onboarding |
| Linux Wayland computer use | Desktop input is explicitly rejected; browser automation is separate |
| Windows protected/elevated desktop or macOS permissions absent | OS capture/input limitations require platform-specific acceptance |
| Read-only/corrupt/incompatible app-data DB | Startup setup propagates failure into `.expect("Unable to initialize LocalLM")`; no in-app repair journey established |
| Unsigned installer / missing WebView2 / upgrade/rollback | Current acceptance absent; historical Windows evidence used an existing configured profile |

The project-folder requirement is current product behavior, not classified here as a defect. Likewise, explicit approval prompts, disabled automation by default, unsupported Wayland input, and credential-store protection are not treated as bugs merely because setup is required.

## Release acceptance gaps

1. **Current clean Windows installer:** Run the current revision on an empty user profile, with no preinstalled model/runtime, and go through first download, first load and first answer. The historical installer test used an existing configured profile and cannot prove this.
2. **Installer trust and lifecycle:** Signing, missing-WebView2 behavior, offline installation, antivirus/quarantine interaction, upgrade, rollback, app-data retention/deletion, and uninstall with an active model need current Windows evidence. No updater/signing pipeline is established in the checked configuration; exact OS prompts were not tested.
3. **Hardware matrix:** At least CPU-only low-RAM, small NVIDIA VRAM, larger NVIDIA, AMD/Intel, multiple GPUs, Apple Silicon, and slow-storage configurations. Verify driver compatibility, acceleration actually used, stable memory demand and useful error recovery. Do not infer these from parser tests.
4. **Real transfer/crash matrix:** Full-size model + projector + split model; disconnect/reconnect, app exit, force-kill, low disk, failed extraction, digest failure and gated-token expiry. Isolated range fixtures do not establish these end-to-end outcomes.
5. **Native credentials and dependencies:** Test real locked/unlocked/unavailable OS keyrings, desktop PATH, old/missing Node, absent browser, and representative authenticated connectors. Unit tests substitute process-memory vault keys.
6. **Backup/startup failure:** Empty/unwritable/full app-data directory, damaged SQLite, newer schema, and restoration on another OS account. The code rejects newer schema; a user-facing recovery workflow still needs acceptance.

## What already works by code/tests

- Pinned/revision-bound download metadata, HTTPS-only transfer client, finite redirect count, streaming byte limits, SHA-256 verification and final-file publication after verification.
- Byte-range validation, saved-prefix rehashing, transfer retries, disk-space reserve checks, cancellation, and preservation of completed final files.
- Split-shard completeness checks and total model-size calculation in GGUF metadata. F21 concerns the separate launch timeout calculation.
- Runtime archive hash/manifest/path validation and private extraction staging.
- Local runtime uses loopback plus a per-process API key, subprocess ownership/cleanup, and health checking. Runtime logs are capped.
- Model context/layer validation and warnings for partial memory estimates; candidate badges explicitly disclaim a guaranteed fit.
- Missing Node guidance, gated-repository token controls, and explicit unsupported managed-runtime messaging.
- Frontend invoke/backend command contracts and the broad passing test suites.

These protections narrow the issues; they do not cancel the findings above.

## Recommended order for a later fix pass

1. **Preserve downloads and make recovery/deletion coherent:** F01, F04–F10. These directly waste user bandwidth and disk space.
2. **Make recommendations trustworthy:** F02, F15–F19. Separate unknown capability from CPU-only, and bind context/package guidance to the actual machine/runtime.
3. **Remove blocking and restore truthful state:** F03, F20–F22.
4. **Finish storage/identity/dependency handling:** F11–F14, F23–F25.
5. **Run current clean-machine release acceptance** before claiming the application is ready for first-time users.

This is a proposed ordering only. No implementation changes have been applied.

## Reproduce the isolated Rust observations

From the repository root, after backend dependencies have been built:

```sh
python3 docs/audits/run-first-run-fixture.py
node docs/audits/run-memory-fixture.mjs
```

The runner compiles `first-run-2026-10-22-fixture.rs` against existing local dependency artifacts and uses a fresh temporary data directory. The fixture copies the audited production scan/cleanup logic; it stubs known catalog provenance for a generic `owner/repo` and uses a minimal runtime manifest with the same archive naming shape. It does not invoke Tauri or touch real application data. It is a snapshot for review, not a regression test against future changes.

Expected observations: startup removes the partial; inventory advertises the missing `.part`; deleting that path returns `NotFound`; a completed weight with only pending metadata loses source/revision; crashed runtime staging rejects `<archive>.zip.part`; a dangling symlink makes inventory return an error. The missing `.part` and deletion failure are one recovery chain, counted as one fixture observation in the five-observation summary above.

The memory runner transpiles and imports the current TypeScript guidance functions, without altering them. Expected output is a 131,072-token recommendation with 12.5 GiB estimated RAM on the 4 GiB-free fixture, and `large` for the two-GPU fixture. Both runners print observations rather than treating the existing defects as passing product acceptance.
