# Bonsai-27B experimental acceptance

Status: the off-by-default feature is wired into Models settings, with a separate
catalog, installer, model directory and PrismML runtime. Acquisition, ten coherent
chat prompts, reasoning and a native read-only tool round-trip have passed.
The five-repeat workload passed and its measured preset is published.
Live streaming and the combined switching/failure recovery acceptance passed.
All final ordinary checks and the 30-minute repeated-chat soak passed. The feature
is qualified on this laptop under the conditions and limits recorded below.
Recorded on 2026-10-05 on branch `feat/experimental-bonsai-27b`.

## Using the opt-in feature

Open Settings → Models (also available in Runtime and Model files), then enable
Experimental models. This reveals Bonsai and its warning. Install the separate
PrismML CUDA 12.4 runtime, download/verify the text weights, then click Use verified
model or Apply measured Bonsai preset. Existing Runtime controls remain available
for overrides; applying the measured preset explicitly replaces those overrides.
Using the model again preserves its saved override profile.

Packaged downloads go below the application's data directory at
`experimental/bonsai-27b/models` and `experimental/bonsai-27b/runtimes`, with each
runtime in a fresh version/UUID directory. Use File details to see the actual
destination. The developer helper uses `.local/experimental/bonsai-27b`; those
verified files are also recognized when there is no managed copy. No projector
is acquired or automatically selected. The model is text only in this feature.

Off by default means the catalog and installation APIs are gated, no experimental
directory is created by the disabled info call, and neither standard runtime nor
MiniCPM catalog changes. Opt-in uses optional keys in the existing JSON settings
table (`experimental_bonsai_enabled` and a previous-selection snapshot). There is
no SQLite migration and no new Preferences or RuntimeConfig schema field.

The first selection saves the previous preferences/configuration; a failed launch
restores and reloads that model when it was previously ready. Disabling while
Bonsai is selected stops it and restores the previous selection; click Load model
to resume it. Download failures retain the existing model selection. A missing or
incomplete fork is refused before changing selection. A failed launch log is
preserved under the experimental root before the normal `runtime.log` is reused.

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

## Final ordinary checks after implementation

| Command | Actual result |
| --- | --- |
| `npm test` | 460 passed, 1 skipped; 80 files passed, 1 skipped; 25.33 seconds |
| `npm run build` | Passed; same existing chunk-size warning |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 415 passed, 4 ignored; main/doc tests passed; 62.71-second test execution |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | Passed; 24.64 seconds |
| `npm run typecheck` | Passed for app and e2e |
| `npm run lint` | Passed, warnings denied |
| `npx playwright test` | 11 passed, Edge; 37.5 seconds |
| `npm run test:scripts` | 39 passed |

One parallel final attempt timed out in the existing Trajectory test, the new
Bonsai test while the lazy Settings module was opening, and the existing session
reload test. Compilation, Vitest, Edge and other suites overlapped in that run.
Full Vitest and Edge reruns passed with no implementation/timeout changes after
compilation finished. This is consistent with resource contention; the exact
cause of timing variability was not independently proven. All original passing
checks passed again. The added real-hardware tests are explicitly ignored in
ordinary runs and were invoked separately as recorded below.

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

### Correction to the requested background assumption

PrismML's [Bonsai 1 guide](https://github.com/PrismML-Eng/Bonsai-demo/blob/f20a62ccb62b201e1690af98b5ef6db8f8b0681f/Bonsai1_README.md#upstream-status-for-binary)
states that binary Q1_0 support has merged into upstream llama.cpp, including
CUDA. The existing stock b10855 source independently defines Q1_0, a 128-weight
block, and CUDA dispatch in `ggml/include/ggml.h`, `ggml/src/ggml-common.h`, and
`ggml/src/ggml-cuda/mmq.cuh`. Therefore the statement that stock llama.cpp cannot
support this format is no longer accurate. This acceptance still uses a separate,
pinned PrismML runtime as requested; it does not establish that a fork is
intrinsically required. Actual ten-prompt acceptance also passed on the unchanged
stock b10855 runtime using the same 4k/Q8/65-layer configuration. Both runtimes
returned coherent factual, Python, math, Spanish, Hindi, JSON and multi-turn
answers; both completed reasoning and the native system_time round-trip. There
was no observed garbage on stock. The dedicated fork remains this opt-in feature's
qualified runtime as requested. This comparison establishes correctness in those
cases, not a throughput ranking (reply lengths differed).
Warnings about newer Bonsai 2 ternary formats must not be applied to this model.

The selected runtime uses the same CUDA version as MiniCPM's runtime. NVIDIA's
[CUDA 12.4 release notes](https://docs.nvidia.com/cuda/archive/12.4.0/cuda-toolkit-release-notes/index.html)
list Windows driver 551.61 for CUDA 12.4 GA and 528.33 for CUDA 12.x minor-version
compatibility, which has restrictions. Both executables start successfully and
report CUDA0 as the RTX 2050, compute capability 8.6. Actual CUDA inference has
passed. No CUDA Toolkit source build was necessary or attempted.

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

## Acquisition and chat evidence

Both runtime archives were downloaded and passed SHA-256 verification, then
extracted with the production bounded extractor. The primary archive has 71
entries (724,707,328 expanded bytes), including `llama-server.exe` and
`llama-bench.exe`; the CUDA DLL archive has 3 entries (574,138,880 expanded bytes).
`--help` for both executables and server `--list-devices` exited 0. No stock
runtime or model files were modified.

`model-cancel` exercised a real fresh transfer and returned `Download cancelled.`
after its 32 MiB threshold; a 33,566,054-byte partial file remained. `model`
resumed it, received all 3,803,452,480 bytes, and verified SHA-256
`17ef842e47450caeb8eaa3ebfbbab5d2f2278b62b79be107985fb69a2f819aa0`
before publishing the final GGUF. No projector or drafter was acquired.

The added `scripts/bonsai-benchmark.mjs` uses only flags discovered in the fork's
help. It samples device-wide NVIDIA memory and process working/private memory
every 500 ms; reported peaks will be sampled lower bounds. The pinned bench
source allocates `n_prompt + n_gen + n_depth` context, so separate pp/tg runs
set depth to hold their requested context constant.

### Initial probe (not a chosen preset)

The completed one-repetition probe used CUDA0, 32 GPU layers, 128 context tokens,
F16 K/V on GPU, Flash Attention on, batch 128 / ubatch 64, six threads and mmap.
Only 16 prompt tokens and eight generated tokens were timed; these tiny runs
establish feasibility and do not validate sustained chat performance.

| Run | Tokens/s | Peak device VRAM (MiB) | Peak process working set (bytes) | Peak private bytes |
| --- | ---: | ---: | ---: | ---: |
| Prompt processing | 10.239828 | 2,003 | 2,663,829,504 | 3,522,445,312 |
| Generation | 5.345935 | 2,011 | 3,930,071,040 | 3,517,919,232 |

Peaks are sampled lower bounds, device VRAM includes other GPU allocations, and
working set/private bytes measure different aspects of process memory. Before
inference, Windows reported only 1,041,432 KiB free physical memory; background
applications remained open. AC power and the Balanced power plan were confirmed.

The ignored Rust test `experimental::acceptance::managed_chat_acceptance` uses
the existing `Runtime::load`/`stop`, authenticated token preflight and embedded
Jinja template, with ten prompts plus reasoning and a native `system_time` tool
round-trip. The production feature is isolated in the experimental Rust module
and the small frontend feature folder; existing touch points are listed below.
The real run passed in 197 seconds with 32 GPU layers, 4096 context, F16 K/V,
batch 128 / ubatch 64, six threads, Flash Attention and mmap. Actual answers:
Paris; H₂O; a valid Python `sum_even` function using `sum` and a generator; 42;
84 = 2×2×3×7; Carol; Madrid in Spanish; H₂O in Hindi; `{"answer":42}`; and
`JUNIPER-4721` retained across the multi-turn conversation. Thinking mode returned
separate nonempty `reasoning_content` and a final answer containing 42.
The model selected the native `system_time` function with
`{"timezone":"Asia/Kolkata"}`; the actual tool result was supplied back and the
model returned the correct date, Monday 5 October 2026. This establishes one
successful read-only tool round-trip, rather than a general tool reliability score.
The ten short prompts disabled thinking; the separate reasoning prompt enabled it.
This was real managed backend inference, not a native GUI click test.

The opt-in UI passed a real Edge test with mocked Tauri APIs (keyboard activation,
persistence and hidden catalog while off). Seven Rust feature tests cover disabled
behavior, catalog/hash/path isolation, incomplete runtime, preserved overrides and
disable restoration. The separate atomic-save regression passed; five frontend
feature tests passed. Strict Clippy passed after integration.

The benchmark sweep completed; the full table below is generated from saved JSON.
The first five-repeat workload attempt found a test-harness padding issue before
generation: repeating a multi-token word could not hit exactly 2000 tokens. The
helper now checks small text suffixes through the actual token-count endpoint.
The failed construction evidence is preserved separately; the rerun below passed.

## Measured preset and five-repeat qualification

Preset: 4096 context, all 65/65 GPU layers, Q8_0 K/V on GPU, Flash Attention on,
batch 128 / ubatch 64, six CPU threads, mmap, one inference slot. The 2k-context
layer run was slightly faster but cannot hold a 2000-token prompt and 500-token
reply. Q8 at 4k led the comparable 4k runs; partial offload, CPU KV, larger F16
caches, disabled Flash Attention and twelve threads were slower in this sweep.
This is the fastest tested candidate that passed the required workload, not proof
of an exhaustive optimum over every parameter combination.

The real authenticated server processed exactly 2000 input and 500 output tokens
five times with no cached input (`cache_prompt:false`), without concurrent builds
or browser tests. All five story outputs were coherent in the sampled passages.
`ignore_eos:true` forced exactly 500 output tokens for measurement; production
chat retains normal EOS behavior. Thinking was disabled for this throughput test.

| Repeat | pp tokens/s | tg tokens/s |
| --- | ---: | ---: |
| 1 | 243.794 | 19.895 |
| 2 | 243.377 | 19.712 |
| 3 | 242.871 | 19.432 |
| 4 | 241.093 | 19.338 |
| 5 | 240.346 | 19.277 |
| Mean | 242.296 | 19.531 |

Sampled peaks: 3855 MiB device VRAM, 4,442,234,880-byte working set (4.137 GiB),
5,431,689,216 private bytes (5.059 GiB). Generation varied by about 3.1% from the
first to fifth repeat while warming; no crash or throughput collapse occurred.
The five requests completed in 169.946 seconds including token preflight and
shutdown, after a successful managed load. AC power and Balanced plan were used;
CUDA0 was the RTX 2050. Runtime status reported actual offload of 65/65 layers.

MiniCPM → Bonsai → MiniCPM → Bonsai passed through an isolated real AppState.
MiniCPM answered `Paris.` before and after the switch; Bonsai answered `Paris`.
The initial unseeded 512-token streaming final-answer check failed; that version
of the harness did not retain its raw stream, so its exact cause is unverified.
The improved harness saves the stream before checking it. Controlled 512- and
2048-token checks both completed 42; the app-settings 2048-token run also completed
42 with separate reasoning and no leaked think markup. That run used 716 generated
tokens, demonstrating that a 512-token reserve can be too short for this prompt.
The normal Bonsai reply reserve is now 2048 tokens, consistent with the existing
UI's guidance for reasoning models. This does not alter the 500-token non-thinking
performance test or MiniCPM's saved settings. Unauthenticated model access returned
401. The existing UI/parser code already handles the observed reasoning field;
no model-specific text or tool parser workaround was added.

The chosen Q8 preset was rerun through all ten prompts, reasoning and the strict
read-only tool round-trip and passed in 47.89 seconds. Stock b10855 passed the same
acceptance in 40.17 seconds; those wall times include replies of different lengths.

## Extended allocation and recovery evidence

At the advertised 262,144-token maximum, Q8 GPU KV allocation failed: CUDA reported
`cudaMalloc failed: out of memory` for an 8704 MiB cache buffer. Q4 GPU KV allocated
4608 MiB and passed one real 2000-input/500-output request: pp 47.419 tokens/s,
tg 9.747 tokens/s. Across those two trials, sampled peaks were 3931 MiB device
VRAM, 6,310,109,184-byte working set and 13,387,485,184 private bytes. Windows may
back CUDA allocations with host memory; a reported CUDA buffer size is not proof
that it all resides in the 4 GB physical VRAM. **This verifies the maximum
allocated window with a 2500-token active history, not a filled 262k history.**
The largest fully populated benchmark depth was 32k, whose tg was only 4.630
tokens/s. Larger-history quality and full-window sustained performance are unverified.

For a forced failure, full offload with F16 at 262k requested a 16,384 MiB CUDA KV
buffer and failed with real CUDA OOM. The UI error identified insufficient GPU
memory and suggested reducing layers/context/batch; the previous MiniCPM settings
were restored, its server reloaded, and it answered `Paris.`. The failed log is
retained as `forced-oom-runtime.log` in local evidence.

Temporarily renaming only the isolated fork executable made activation fail with
the missing/incomplete-runtime error before changing selection or stopping the
ready MiniCPM server. The executable was restored immediately. This tests the
missing-file condition without destroying the verified release bundle.

A real process kill after nonempty streamed reply text interrupted the HTTP stream.
The first immediate status assertion raced Windows exit notification. The bounded
poll rerun detected `phase:error` after 439 ms, cleared offload reporting, restored
MiniCPM through opt-out/reload, and got `Paris.` again. This harness correction did
not change the existing runtime process manager. The combined acceptance rerun
passed in 397.93 seconds: MiniCPM/Bonsai switching, reasoning SSE, authentication,
forced OOM restoration, missing runtime refusal and real process-kill recovery
all passed in one run. That kill was recognized after 505 ms; MiniCPM again
answered `Paris.`. The combined negative-case sample peaked at 3909 MiB device
VRAM, 5,530,456,064-byte working set and 20,985,253,888 private bytes, dominated
by the intentionally oversized allocation. These are not the chosen preset's
ordinary memory requirements. Individual kill recovery passed in 19.09 seconds.

## Completed 30-minute soak

`node scripts/bonsai-qualification.mjs soak` passed with 53 complete requests
over 1829.362 seconds (30 minutes 29 seconds, including shutdown), after a
successful managed load. The request loop recorded 1829.079 seconds. Every
request had exactly 2000 input and 500 output tokens; all returned nonempty,
well-formed text and the server remained ready. First, middle and last saved
story passages were coherent. Normal prompt caching was enabled, but the changed
record prefix meant the server reported zero cached input tokens in every round.
Thinking was disabled and `ignore_eos:true` forced the measurement length, as in
the five-repeat qualification. This is repeated chat, not a 30-minute filled
context or general quality test.

| Measurement | Actual result |
| --- | ---: |
| Mean prompt processing | 236.376 tokens/s |
| Mean generation | 19.209 tokens/s |
| Generation range | 18.592–19.913 tokens/s |
| First five / last five mean generation | 19.637 / 18.907 tokens/s |
| Sampled peak device VRAM | 3855 MiB |
| Sampled peak resident working set | 4,436,283,392 bytes (4.132 GiB) |
| Sampled peak private commitment | 5,430,984,704 bytes (5.058 GiB) |
| Median working set, minutes 5–10 / final 5 minutes | 3.889 / 3.738 GiB |
| Median private commitment, minutes 5–10 / final 5 minutes | 5.056740 / 5.057693 GiB |

The first-to-last five-request mean fell by 3.72%; the final three were slower
than the earlier plateau. The cause of that variation was not measured. There
was no crash or throughput collapse. Median private commitment increased by
0.977 MiB between the comparison windows, while resident memory decreased.
No sustained leak was detected in this bounded test; this does not prove absence
of leaks over longer runs. Device memory returned to 8 MiB after shutdown and no
llama-server process remained. The laptop still reported AC power and the same
Balanced plan; no builds or browser tests ran alongside the soak. Raw records,
500 ms samples and the summary are retained in `.local/bonsai-evidence/soak.json`,
`qa-soak-memory.json`, `qa-soak-result.json` and `soak-summary.json`.

<!-- BENCHMARK TABLE START -->
## Complete benchmark sweep

Layer runs timed pp128 / tg64 once; the tiny probe timed pp16 / tg8 once.
All other runs time pp512 / tg128, three repetitions each. The remainder of
each context is prefetched as depth and excluded from the timed token count.
Every completed row below comes directly from the saved benchmark JSON.
Background applications remained open; some build/test work ran during the
exploratory sweep. The selected preset is separately qualified with real chat
without concurrent builds or browser tests. Keep AC connected for comparison.

| Case | ctx / GPU layers | K=V / KV placement / FA | batch / ubatch / threads / load | pp tok/s | tg tok/s | peak device MiB | peak working set GiB | peak private GiB |
| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| probe | 128 / 32 | f16 / GPU / on | 128 / 64 / 6 / mmap | 10.240 | 5.346 | 2011 | 3.660 | 3.281 |
| layers-65 | 2048 / 65 | f16 / GPU / on | 128 / 64 / 6 / mmap | 246.793 | 20.302 | 3889 | 4.094 | 5.044 |
| layers-56 | 2048 / 56 | f16 / GPU / on | 128 / 64 / 6 / mmap | 169.260 | 9.455 | 3449 | 4.273 | 4.798 |
| layers-48 | 2048 / 48 | f16 / GPU / on | 128 / 64 / 6 / mmap | 149.665 | 7.130 | 3011 | 4.306 | 4.403 |
| layers-40 | 2048 / 40 | f16 / GPU / on | 128 / 64 / 6 / mmap | 134.738 | 5.049 | 2579 | 4.339 | 4.012 |
| layers-32 | 2048 / 32 | f16 / GPU / on | 128 / 64 / 6 / mmap | 115.044 | 4.144 | 2143 | 4.372 | 3.620 |
| layers-16 | 2048 / 16 | f16 / GPU / on | 128 / 64 / 6 / mmap | 100.502 | 2.976 | 1273 | 4.438 | 2.836 |
| layers-0 | 2048 / 0 | f16 / GPU / on | 128 / 64 / 6 / mmap | 85.431 | 2.746 | 453 | 4.465 | 2.058 |
| ctx4096-f16 | 4096 / 65 | f16 / GPU / on | 128 / 64 / 6 / mmap | 48.457 | 14.368 | 3909 | 4.318 | 5.282 |
| ctx4096-q8 | 4096 / 65 | q8_0 / GPU / on | 128 / 64 / 6 / mmap | 239.204 | 19.202 | 3895 | 4.095 | 5.049 |
| ctx4096-q4 | 4096 / 65 | q4_0 / GPU / on | 128 / 64 / 6 / mmap | 237.674 | 18.894 | 3831 | 4.035 | 4.925 |
| ctx8192-f16 | 8192 / 65 | f16 / GPU / on | 128 / 64 / 6 / mmap | 44.386 | 9.421 | 3897 | 4.744 | 5.785 |
| ctx8192-q8 | 8192 / 65 | q8_0 / GPU / on | 128 / 64 / 6 / mmap | 48.041 | 13.105 | 3903 | 4.357 | 5.315 |
| ctx8192-q4 | 8192 / 65 | q4_0 / GPU / on | 128 / 64 / 6 / mmap | 230.072 | 17.670 | 3903 | 4.111 | 5.068 |
| ctx16384-q4 | 16384 / 65 | q4_0 / GPU / on | 128 / 64 / 6 / mmap | 29.794 | 10.536 | 3915 | 3.986 | 5.353 |
| ctx32768-q4 | 32768 / 65 | q4_0 / GPU / on | 128 / 64 / 6 / mmap | 42.309 | 4.630 | 3931 | 3.966 | 5.921 |
| cpu-kv-q8 | 4096 / 65 | q8_0 / CPU / on | 128 / 64 / 6 / mmap | 181.834 | 8.525 | 3625 | 4.380 | 5.072 |
| flash-off-f16 | 4096 / 65 | f16 / GPU / off | 128 / 64 / 6 / mmap | 45.531 | 11.829 | 3929 | 4.485 | 5.487 |
| batch64-ub32 | 4096 / 65 | f16 / GPU / on | 64 / 32 / 6 / mmap | 48.179 | 14.439 | 3911 | 4.286 | 5.254 |
| batch256-ub128 | 4096 / 65 | f16 / GPU / on | 256 / 128 / 6 / mmap | 51.381 | 14.065 | 3911 | 4.381 | 5.350 |
| batch512-ub256 | 4096 / 65 | f16 / GPU / on | 512 / 256 / 6 / mmap | 53.223 | 14.422 | 3913 | 4.509 | 5.481 |
| threads12 | 4096 / 65 | f16 / GPU / on | 128 / 64 / 12 / mmap | 50.176 | 13.447 | 3909 | 4.304 | 5.283 |
| no-mmap | 4096 / 65 | f16 / GPU / on | 128 / 64 / 6 / none | 50.022 | 14.141 | 3909 | 1.107 | 5.444 |
| mlock | 4096 / 65 | f16 / GPU / on | 128 / 64 / 6 / mlock | 48.566 | 14.073 | 3909 | 0.805 | 5.445 |

VRAM is device-wide, including other applications. Memory is sampled every
500 ms and peaks are lower bounds. Working set is resident process memory;
private bytes measure committed memory and must not be called physical RAM.
These configurations did not use the server prompt cache.
<!-- BENCHMARK TABLE END -->

## File inventory

Existing files modified by this task (other pre-existing working-tree changes
are outside its scope):

| File | Purpose |
| --- | --- |
| `src-tauri/src/commands.rs` | Resolve the owned model's runtime and restore the previous selection after an experimental reload fails. |
| `src-tauri/src/lib.rs` | Register seven opt-in commands and initialize its isolated installer state. |
| `src-tauri/src/model_library.rs` | Read the existing saved per-model load profile, preserving overrides. |
| `src-tauri/src/runtime.rs` | Bound prompt-cache RAM for the owned PrismML runtime only. |
| `src-tauri/src/runtime_install.rs` | Reuse the verified archive installation flow with a separate manifest and destination. |
| `src-tauri/src/store.rs` | Atomically save the model selection and runtime configuration in existing JSON settings; no migration. |
| `src-tauri/src/web_search.rs` | Add the installer field to a pre-existing, uncommitted test fixture. This one compatibility line remains unstaged to avoid committing unrelated user work. |
| `src/lib/api.ts` | Add typed native experimental command adapters. |
| `src/components/Models.tsx` | Show the opt-in section and refresh selection after activation/recovery. |
| `src/components/ModelDownload.tsx` | Reuse existing progress, disk-space, cancel/retry and verification controls with an optional backend and copy. |
| `src/components/RuntimeDownload.tsx` | Reuse existing runtime installation controls with an optional backend and separate-runtime copy. |
| `docs/RUNTIME.md` | Record runtime provenance, build workflow, licenses and isolated launch behavior. |
| `docs/PROGRESS.md` | Record measured acceptance results and their limits. |

New files:

- `catalog/bonsai-27b.json`, `catalog/bonsai-runtime-assets.json`: immutable model/runtime metadata, measured preset and exact extraction inventory.
- `src-tauri/src/experimental/mod.rs`, `bonsai.rs`: opt-in state, isolated paths, verified installation, model-specific selection and recovery.
- `src-tauri/src/experimental/acceptance.rs`, `qualification.rs`: explicitly ignored real-hardware chat, workload, streaming, maximum allocation and failure acceptance.
- `src-tauri/examples/bonsai_acquire.rs`: isolated acquisition using production download/extraction code.
- `src/features/bonsai/BonsaiSettings.tsx`, `BonsaiSettings.test.tsx`, `types.ts`: existing design-system opt-in controls, tests and typed metadata.
- `scripts/bonsai-benchmark.mjs`, `scripts/bonsai-qualification.mjs`: explicit hardware measurements and process-memory sampling.
- `e2e/bonsai.spec.ts`: Edge opt-in visibility/persistence acceptance.
- `docs/BONSAI_EXPERIMENTAL.md`: this evidence and removal report.

## Reproducing the checks

Ordinary checks do not download or load Bonsai. Run from the repository root:

```powershell
npm test
npm run build
npm run typecheck
npm run lint
npm run test:scripts
npx playwright test
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Explicit acquisition and hardware commands actually run on this laptop:

```powershell
cargo run --manifest-path src-tauri/Cargo.toml --example bonsai_acquire -- runtime
cargo run --manifest-path src-tauri/Cargo.toml --example bonsai_acquire -- model-cancel
cargo run --manifest-path src-tauri/Cargo.toml --example bonsai_acquire -- model
node scripts/bonsai-benchmark.mjs probe
node scripts/bonsai-benchmark.mjs layers
node scripts/bonsai-benchmark.mjs matrix
node scripts/bonsai-qualification.mjs workload
node scripts/bonsai-qualification.mjs max-context
node scripts/bonsai-qualification.mjs streaming
node scripts/bonsai-qualification.mjs kill
node scripts/bonsai-qualification.mjs integration
```

`soak` uses the same qualification driver for at least 1800 seconds of repeated
chat after loading. Hardware modes require `candidate-preset.json` in the ignored
evidence directory. To reproduce with the published preset after acquisition,
create that file from the catalog (the original qualification used the same
configuration):

```powershell
node -e "const fs=require('node:fs'); fs.mkdirSync('.local/bonsai-evidence',{recursive:true}); const c=JSON.parse(fs.readFileSync('catalog/bonsai-27b.json','utf8')); fs.writeFileSync('.local/bonsai-evidence/candidate-preset.json',JSON.stringify(c.preset));"
node scripts/bonsai-qualification.mjs soak
```

The chat acceptance was also run with
`BONSAI_ACCEPTANCE_PRESET=1`, and again with `BONSAI_ACCEPTANCE_RUNTIME=stock`,
using `cargo test --manifest-path src-tauri/Cargo.toml --lib
experimental::acceptance::managed_chat_acceptance -- --ignored --nocapture`.
The driver records raw responses, logs, exact CLI arguments, memory samples and
exit status under `.local/bonsai-evidence`. Release help, HF/GitHub API metadata,
hardware/power-plan probes and the existing default artifact hashes were also
captured there. Hardware modes are explicit and excluded from ordinary tests.

## Limits of this acceptance

The ten prompts and one successful read-only tool call are small acceptance
samples, not a broad quality, multilingual or tool-reliability benchmark.
The 4k preset's normal 2048-token reply reserve includes reasoning and leaves
about 2048 tokens for input. Large tool lists or long system instructions can
require a larger context or smaller reserve through existing Runtime/Generation
controls. The throughput qualification used a 500-token non-thinking reply.
The maximum context allocation does not establish correctness at a filled 262k
history. Memory samples are lower bounds; private commitment is not physical RAM.
The Edge test exercised actual browser UI with mocked IPC; hardware acceptance
exercised actual managed Rust/server paths with isolated temporary databases.
The combined native GUI download/click path, packaged installer and another
machine/older driver were not tested. Checks run against the existing dirty
working tree. The task's code uses the user's pre-existing uncommitted
`Settings.tsx` components and shared Rust test fixture; those files were not
swept into this task's commits. A checkout containing only the task commit stack
is therefore not a self-contained snapshot of the tested tree. Reconcile/commit
the user's baseline work before expecting standalone checkout checks. That
checkout has not been validated.

The existing MiniCPM GGUF still hashes to its pinned
`d39e78a06dbb9b28ed9a9118b1370e992caf862003c925264ea33789b3e416bc`
(2,070,227,904 bytes). The unchanged stock server identifies b10855-7d701b592;
its recorded post-test executable hash is
`ebede3340ade0cb80db0929d1854e343e522267b7a30eb28c7ad9d7cf1090e32`.
The task's acquisition/install code never writes those default paths. No initial
per-DLL fingerprint was captured, so this is not a claimed complete before/after
runtime-bundle checksum comparison. Real switching/recovery used the original
MiniCPM path and runtime and preserved its 32k saved/default load configuration.

## Disable and remove

1. Turn off Experimental models. If Bonsai is selected, this stops its process
   and restores the saved previous model/settings. Click Load model to resume
   MiniCPM. Check the displayed model/runtime paths before removing files.
2. With Bonsai stopped, remove only the application's data-directory
   `experimental/bonsai-27b` tree and the developer
   `.local/experimental/bonsai-27b` tree if present. File details shows the managed
   destination. `.local/bonsai-evidence` is optional evidence, removable separately.
   Preserve `.local/runtime`, `.local/models` and the standard app-data model/runtime
   directories. No files were deleted as part of this acceptance.
3. The optional `experimental_bonsai_enabled` and
   `experimental_bonsai_previous` JSON setting keys can remain inert. Complete
   metadata cleanup may remove only those two keys and the owned Bonsai model's
   saved profile through existing settings/profile storage; no schema rollback
   or conversation deletion is required.
4. To remove the implementation, revert only this task's commits in reverse
   order, preserving unrelated working-tree changes. Git may require resolving
   overlap with the user's existing work. Also remove only the added
   `bonsai_installer: Default::default()` compatibility line from the pre-existing
   `web_search.rs` fixture; do not restore that whole file.

The task started from commit `5eb2ba3`; its first commit is `43dc035`. At delivery,
`git log --oneline 5eb2ba3..feat/experimental-bonsai-27b` identifies the task
commits and `git diff 5eb2ba3..feat/experimental-bonsai-27b` shows only their
committed changes. Review that list before reverting if more work has since
landed on the branch. The dirty working tree additionally contains the preserved
user work and the one compatibility line described above.
