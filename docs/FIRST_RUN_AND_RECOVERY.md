# First run and recovery

## Get to the first answer

1. Add a project folder. LocalLM requires a project for a conversation.
2. Open **Models → Model files**. Choose a storage location before downloading if the default volume is short on space. **Change storage location** copies managed models and runtimes to an empty folder, verifies each copy, migrates saved paths/profiles, and switches the persistent location only after copying. Unload models and finish or cancel installations first.
3. Install/select a native `llama-server` matching the operating system, CPU architecture and acceleration backend. Save its executable path. Keep its matching libraries alongside it.
4. Discover a GGUF and read its publisher's architecture, quantization and projector requirements. File integrity and runtime compatibility are separate: a verified download is not a promise of loadability. The selected runtime is checked before a Hub transfer when configured; known missing projector support is rejected. Architecture/quantization/projector pairing remains unverified until the runtime successfully loads it.
5. Choose smaller weights when memory is tight. Runtime suggestions reserve RAM for weights, cache, projector, compute buffers and headroom; they reduce context rather than choosing the model's training maximum. When even minimum context is too large, or telemetry/cache layout is unknown, **Use recommended settings** is unavailable. Manual settings remain available for informed use.
6. Use the downloaded model. Loading reports elapsed time and readiness checks; **Cancel model loading** stops the owned process. Split models use total shard size for the startup budget. Check the authoritative runtime status after a failed replacement: an existing working model can remain loaded.

### Runtime choices

The only automatically downloaded runtime remains the catalog-pinned Windows x64 CUDA bundle. It is never offered to Windows ARM64, Linux or macOS. CUDA libraries in that bundle do not establish GPU/driver compatibility. Its CPU libraries do not imply AMD/Intel acceleration.

For other combinations, obtain a native release or build from the official project and select it with **Browse runtime**:

| Machine/backend | Native setup alternative |
|---|---|
| Windows/Linux CPU, including architecture-specific machines | Matching CPU release or CPU build |
| NVIDIA | CUDA build and its documented dependencies |
| AMD | HIP/ROCm or Vulkan build, according to upstream support |
| Intel GPU | Vulkan or SYCL build, according to upstream support |
| Apple Silicon / Intel macOS | Matching native macOS release/build; Metal where supported |

Consult the [official build instructions](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md) and [pinned upstream release](https://github.com/ggml-org/llama.cpp/releases/tag/b10855). Follow their current platform requirements; LocalLM does not invent download URLs, checksums or driver minimums for additional packages. No additional managed package is claimed as supported or validated in this change.

**Models → Diagnostics → Inspect runtime devices** queries the saved executable. NVIDIA memory readings do not enumerate every Metal/Vulkan/ROCm device. Automatic placement belongs to the runtime. Legacy numeric CUDA fitting is used only when the selected executable reports the same sole NVIDIA device; mismatches and other device lists remain unknown/automatic. With multiple devices or missing readings, per-device fit is unknown; discovery badges are only candidates, not reservations or a guarantee about the selected runtime. Memory samples refresh periodically and expire after 30 seconds.

## Interrupted downloads and deletion

Cancellation, ordinary quit and restart preserve model `.part` files. Resume reuses prefixes and completed shards, validates immutable source metadata, rehashes the prefix, and verifies the final file. Completed weights with a pending projector stay incomplete and retain the repository-relative source names.

Recovery journals use synced atomic replacement and keep a previous valid journal. An interrupted write can fall back to that copy. If both are malformed, retained orphan weight prefixes remain visible for deletion, with unknown source identity. Recover the original immutable source before attempting a manual repair; do not guess a new revision for old bytes. Legacy catalog partials resume through their pinned installer.

**My Models → Delete** tolerates already-missing files. It removes owned weights, projector, partials and journals. Projectors selected from external/shared locations are not deleted. Interrupted entries display the actual recorded bytes on disk in the deletion prompt. Nothing automatically discards unknown model prefixes on startup. Recognized abandoned runtime archive staging can be cleaned safely; unexpected content/links are retained with an error.

Publication uses an atomic create-if-absent hard link. On volumes that cannot provide this guarantee, LocalLM retains the verified prefix and asks for a supporting volume. It never falls back to a potentially overwriting rename.

## Storage changes and runtime repair

The storage pointer lives in app data as `managed-storage.json`; conversations, credentials and profiles stay in app data. Existing installations default to their original location. A failed copy does not switch locations or delete the source. To recover after interruption, choose the same destination again: matching completed copies are verified/reused, and differing files are rejected. Keep enough reserve space and reconnect removable volumes before starting LocalLM. A malformed pointer fails explicitly instead of silently using another location.

The original storage tree is retained after a successful change. This deliberately requires temporary extra space. Test loading and resuming from the new location before removing the old copy manually. Do not delete app-data conversations, credentials, profiles or the storage pointer when reclaiming the old `models`/`runtimes` trees. External paths remain external. Keep a backup when migrating across disks.

New managed runtime installations have a receipt derived from the verified extracted bytes. Install reuses a complete bundle only when its hashes still match. Legacy bundles without a receipt remain selectable but are not silently certified. To repair, unload the model, remove the incomplete bundle from **Installed runtimes**, then install again. Removal refuses unexpected files or links and protects runtime use through the operation gate. It does not delete arbitrary manually selected executables.

## Prerequisites and failure recovery

- **Insufficient disk:** choose another managed location or free space. Downloads reserve verification overhead; migration retains the original and needs space for the copy. Mid-transfer errors preserve resumable bytes.
- **Offline:** use preprovisioned native runtime/model files, or reconnect before managed provisioning. No model/runtime is bundled. Native/local conversation features do not require cloud credentials.
- **Gated repository:** accept the publisher's license with the correct Hub account and supply a read token. A token does not bypass an unaccepted license. Retry the same immutable revision after connectivity returns.
- **Missing/old Node:** web/weather workers require Node 22.13+; browser package setup requires Node 18+. LocalLM executes a bounded version/behavior probe. Configure an absolute executable path in Execution settings; a desktop app may not inherit the terminal's version-manager PATH.
- **Missing browser:** install the selected Chrome/Edge channel or set its executable path. Diagnostics report package, Node and selected-browser results separately. A package alone is not a ready browser. A version probe is not interactive desktop acceptance.
- **Credential store unavailable:** unlock/start the OS credential store (Secret Service on Linux) and retry. Reads/writes run on blocking workers without a shared database guard. Failed provider key changes restore previous metadata and leave the prior sealed key intact. OS calls cannot safely be abandoned while they may still commit a key update; there is no fake timeout/success. Backups moved to another OS account may not contain the protected encryption key.

## Platform acceptance still required

Use a disposable OS account and isolated storage. Do not reuse real conversations, tokens, model installations or credentials for destructive checks.

1. On clean Windows x64, Windows ARM64, Linux x64/ARM64, Apple Silicon and Intel macOS, follow the native setup path through first answer. Confirm correct executable architecture, installer permissions and prerequisite messages. Validate signing/WebView2/upgrade/uninstall separately.
2. Test CPU-only low RAM, NVIDIA small/large VRAM, AMD/Intel acceleration and multiple GPUs. Query runtime devices; confirm logs show the intended backend and layers. Consume memory in another application, wait for refreshed samples, and verify that stale/unknown estimates are not offered as safe recommendations.
3. Transfer full-size weights, split shards and projector. Cancel, quit/reopen, disconnect, force-kill and fill an isolated volume. Resume and compare final hashes. Delete after a shard/projector has been manually removed. Repeat deletion. Kill during a runtime `.zip.part` download; reopen and check staging cleanup.
4. Move storage across real volumes. Interrupt the copy and restart it, disconnect/reconnect the destination, inject full-disk and write failures, and load models with migrated profiles. Verify executable permissions and case-sensitive/case-insensitive profile behavior on the target filesystem. Test unsupported hard-link publication (for example a suitable FAT/exFAT volume) and an existing destination.
5. Load a slow large split model, poll status concurrently, cancel, and confirm its process tree exits. Reject a replacement before startup and confirm the working model's identity/configuration remain displayed.
6. Exercise locked/unlocked/unavailable real OS keyrings while listing conversations and saving/deleting provider credentials. Test old/missing Node and absent Chrome/Edge in a graphical session. These are native acceptance checks, not claims made by Linux unit tests.

See the [finding tracker](audits/FIRST_RUN_FIX_TRACKER_2026-10-22.md) for implementation evidence and exact automated validation.
