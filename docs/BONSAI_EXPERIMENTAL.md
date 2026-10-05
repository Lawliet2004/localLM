# Bonsai-27B experimental acceptance

Status: the runtime has been acquired and verified; model transfer and inference
feasibility checks are in progress. **The application feature has not been enabled
or integrated. No preset or local throughput is validated.**
Recorded on 2026-10-05 on branch `feat/experimental-bonsai-27b`.

## Baseline before changes

| Command | Actual result |
| --- | --- |
| `npm test` | 455 passed, 1 skipped; 79 test files passed, 1 skipped |
| `npm run build` | Passed; existing warning about a chunk larger than 500 kB |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 407 passed, 2 ignored; main/doc tests passed |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | Passed |
| `npm run typecheck` | Passed |
| `npm run lint` | Passed |
| `npx playwright test` | 10 passed, Edge channel |
| `npm run test:scripts` | 39 passed |

The working tree already contained extensive unrelated changes. They are preserved
and excluded from this task's commits. The frontend's canvas diagnostic, Vite chunk
warning, and Node SQLite/colour-environment warnings did not fail the baseline.

## Current application flow

1. The managed Rust catalog pins MiniCPM's filename, revision, URL, bytes and hash.
2. The Hugging Face library separately resolves immutable revisions and LFS hashes.
3. Managed models use app-data `models`; developer weights use `.local/models`.
4. Downloads check free disk space with a 256 MiB reserve before transfer.
5. Cancellation keeps partial transfers for Range retry; size and hash gate use.
6. Runtime archives have pinned hashes and exact filename/size extraction manifests.
7. Runtime installations stage privately and publish into fresh versioned directories.
8. The managed server uses an authenticated loopback endpoint and bounded runtime log.
9. SQLite stores settings as JSON; separate model profiles retain paths and load settings.
10. Loading stops the old server before spawning its replacement.

## Verified upstream metadata

Model metadata was read from the [Hugging Face API](https://huggingface.co/api/models/prism-ml/Bonsai-27B-gguf?blobs=true).
The [pinned model card](https://huggingface.co/prism-ml/Bonsai-27B-gguf/blob/f10afb355f104535e3e3e98cf7ab7795c72bd292/README.md)
describes Q1_0_g128 (1.125 bits/weight), a Qwen3.6-27B hybrid-attention backbone,
and optional vision/drafter files. This task uses the text weights only.

| Item | Pinned value |
| --- | --- |
| Repository | `prism-ml/Bonsai-27B-gguf` |
| Revision | `f10afb355f104535e3e3e98cf7ab7795c72bd292` |
| Filename | `Bonsai-27B-Q1_0.gguf` |
| Bytes | 3,803,452,480 |
| LFS SHA-256 | `17ef842e47450caeb8eaa3ebfbbab5d2f2278b62b79be107985fb69a2f819aa0` |
| Model license | Apache-2.0, from API and model card |

Runtime assets were discovered through the [GitHub releases API](https://api.github.com/repos/PrismML-Eng/llama.cpp/releases)
and the [release page](https://github.com/PrismML-Eng/llama.cpp/releases/tag/prism-b10754-2459f68).

- Source: `https://github.com/PrismML-Eng/llama.cpp`
- Release: `prism-b10754-2459f68` (published 2026-10-02).
- Commit: `2459f68b5c0eb26261fd5a81682004b93cd645ba`.
- Windows x64 CUDA 12.4 archive: `llama-prism-b10754-2459f68-bin-win-cuda-12.4-x64.zip`,
  312,971,133 bytes, API SHA-256 `200842b4c689fe092a8a1684ef1e185d8d025a0bfae6908737607a0f29560b96`.
- CUDA DLL archive: `cudart-llama-bin-win-cuda-12.4-x64.zip`, 391,443,627 bytes,
  API SHA-256 `8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6`.
- Runtime license: MIT, verified in the pinned source `LICENSE`.
- Pinned CUDA source includes `GGML_TYPE_Q1_0` dispatch and 128-weight-block
  dot-product kernels in `ggml/src/ggml-cuda/mmq.cuh` and `vecdotq.cuh`.

The selected runtime uses the same CUDA version as MiniCPM's runtime. NVIDIA's
[CUDA 12.4 release notes](https://docs.nvidia.com/cuda/archive/12.4.0/cuda-toolkit-release-notes/index.html)
list Windows driver 551.61 for CUDA 12.4 GA and 528.33 for CUDA 12.x minor-version
compatibility, which has restrictions. Both executables start successfully and
report CUDA0 as the RTX 2050, compute capability 8.6. Inference compatibility is
still pending. No CUDA Toolkit source build has been attempted.

## Machine and measurement conditions

`nvidia-smi --query-gpu=name,driver_version,compute_cap,memory.total,memory.free --format=csv`
reported RTX 2050, driver 592.82, compute capability 8.6, 4096 MiB total,
3955 MiB free before acquisition. CPU: Ryzen 5 7535HS, 6 cores / 12 threads.
Free C: space at preflight: 76,889,665,536 bytes.
The laptop reported AC power connected and the Balanced power plan. Benchmarks
must record those conditions and confirm CUDA0 is the NVIDIA dGPU.

## Acquisition helper

`cargo run --manifest-path src-tauri/Cargo.toml --example bonsai_acquire -- runtime`
uses the existing production Rust downloader and runtime extractor. It checks the
pinned bytes and SHA-256, extracts into private staging, and creates a fresh
directory below `.local/experimental/bonsai-27b`. It never selects an application
model or changes settings. `model` downloads/verifies only the Q1_0 weights into
that same isolated root. Archive contents and extraction manifests are discovered
from the verified ZIPs, rather than invented.

## Acceptance still required

Both runtime archives were downloaded and passed SHA-256 verification, then
extracted with the production bounded extractor. The primary archive has 71
entries (724,707,328 expanded bytes), including `llama-server.exe` and
`llama-bench.exe`; the CUDA DLL archive has 3 entries (574,138,880 expanded bytes).
`--help` for both executables and server `--list-devices` exited 0. No stock
runtime or model files were modified.

`model-cancel` exercised a real fresh transfer and returned `Download cancelled.`
after its 32 MiB threshold; the partial file remained and `model` restarted it.
The complete model has not yet been published or hash verified.

The added `scripts/bonsai-benchmark.mjs` uses only flags discovered in the fork's
help. It samples device-wide NVIDIA memory and process working/private memory
every 500 ms; reported peaks will be sampled lower bounds. The pinned bench
source allocates `n_prompt + n_gen + n_depth` context, so separate pp/tg runs
set depth to hold their requested context constant. No performance run is claimed.

The ignored Rust test `experimental::acceptance::managed_chat_acceptance` uses
the existing `Runtime::load`/`stop`, authenticated token preflight and embedded
Jinja template, with ten prompts plus reasoning and a native `system_time` tool
round-trip. Its test-only module is the only current application source touch.
It has compiled and passed strict Clippy; the real model run is pending.

Full model transfer/hash verification, benchmark
sweep, coherent chat and reasoning/tool acceptance, completed resume,
failure recovery, model switching, 30-minute soak, opt-in integration and its
automated checks are pending. No benchmark numbers or stable preset are claimed.

## Removal at this stage

Revert this task's commits to remove its documentation and acceptance helper.
With no experimental processes running, remove only
`.local/experimental/bonsai-27b` and `.local/bonsai-evidence` to remove its
downloaded artifacts and local evidence. Existing `.local/runtime`,
`.local/models`, app settings, SQLite schema and MiniCPM catalog are unchanged.
