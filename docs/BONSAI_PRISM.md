# Bonsai 2 9B with Prism

In **Settings → Models & runtime → Models**, use **Install and use Bonsai**.
The application installs the pinned PQ2_0 model with its integrated MTP head
and a compatible Prism build in managed storage, then loads it. On NVIDIA
hardware it proposes CUDA; CPU remains available. Apple Silicon uses Metal.
The initial profile uses 16,384 context tokens, F16 KV cache, and an 8,192-token
response reserve. It starts with text input. A matching vision projector can
be configured separately in Model files.

The model and runtime archives are SHA-256 verified before publication.
Runtime bundles use separate versioned directories. Reusing a runtime rehashes
its extracted files against its receipt. Downloads do not block ongoing chat;
activation waits for the current operation. Cancellation belongs to this
installation, including its activation wait and startup.

If startup fails, verified downloads remain on disk and the previous saved
selection is restored. If startup had already stopped the previous process,
load that selected model again to resume it. Use **Install and run without MTP**
to retry Bonsai without speculative decoding. For an already selected Bonsai,
**Runtime → Multi-token prediction → Off**, then save configuration, does the
same. Installation preserves other runtime bundles; deletion remains explicit.

## Pinned inputs and maintenance

- Model: `prism-ml/Ternary-Bonsai-2-9B-gguf`, revision
  `5b2abb02362efbed46bbe204014b02837778b2c2`.
- File: `Ternary-Bonsai-2-9B-PQ2_0-MTP-Q8_0.gguf`, 2,675,410,720 bytes.
- Runtime: Prism `prism-b10826-c9c34bc`.
- Managed packages: Linux x64 CPU/CUDA 12.4, Windows x64 CPU/CUDA 12.4,
  and macOS ARM64 Metal. CUDA requires a compatible installed NVIDIA driver.
- Runtime metadata: `catalog/prism-runtimes.json` and `runtime_catalog.rs`.
- Model metadata: `model_catalog.rs`; matching picker label: `localModels.ts`.
- Profile and orchestration: `bonsai.rs`; launch validation: `runtime.rs`.

Follow the publisher's [model card](https://huggingface.co/prism-ml/Ternary-Bonsai-2-9B-gguf/blob/5b2abb02362efbed46bbe204014b02837778b2c2/README.md)
and [MTP usage](https://huggingface.co/prism-ml/Ternary-Bonsai-2-9B-gguf/blob/5b2abb02362efbed46bbe204014b02837778b2c2/USAGE.md).
MTP uses `--spec-type draft-mtp --spec-draft-n-max 2 --spec-draft-n-min 0
--spec-draft-p-min 0`. The combined file is a standalone model, not a companion
head. Auto enables MTP only for that exact profile; other GGUFs receive no MTP
arguments. Local Bonsai requests use top-k 20, min-p 0, repetition penalty 1,
and a 64-token penalty window. Presence penalty follows the request's thinking
control. User temperature/top-p preferences remain authoritative. Remote API
requests do not receive these overrides.

Changing a runtime pin requires updating the package checksums and the version
check in `runtime.rs`, then rerunning archive, inventory and launch regression
tests. Prism archives are pinned as complete containers. Extraction rejects
unsafe paths and file types, limits expansion to 2 GiB and 1,000 flat files,
and materializes internal shared-library aliases as ordinary files. Executable
permissions are retained for native Unix bundles. The older standard Windows
installer remains available.

## Verification boundary

Unit tests cover profile scoping, compatibility flags, cancellation ownership,
backward-compatible saved configuration, receipt tampering, archive integrity,
unsafe aliases, disk-space UI and explicit MTP-off retry. The cached Linux CPU
smoke test runs the production extraction path and checks `llama-server --help`:

```sh
LOCALLM_PRISM_ARCHIVE=/path/to/llama-prism-b10826-c9c34bc-bin-ubuntu-x64.tar.gz \
  cargo test --manifest-path src-tauri/Cargo.toml --lib \
  cached_prism_bundle_extracts_and_exposes_mtp_controls -- --ignored
```

This smoke test does not perform tensor inference. Full Bonsai generation,
MTP acceptance/performance, tool calling, vision, CUDA, Windows and Metal need
hardware validation with the actual weights. Runtime readiness reports accepted
startup settings, not a measured MTP speedup. Browser preview was not used.
