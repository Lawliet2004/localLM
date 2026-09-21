# Runtime provenance

Development runtime: ggml-org/llama.cpp release b10855, Windows x64 CUDA 12.4. Archives and SHA-256 digests are pinned in scripts/prepare-runtime.ps1 using GitHub release asset metadata.

llama-server is launched with `--parallel` from Runtime **Inference slots** (default 1). Local chat completions send `cache_prompt: true` and `id_slot: 0` for the parent conversation; subagents use `id_slot: 1` when two slots are enabled. Token counting (`/v1/chat/completions/input_tokens`) omits both fields so preflight cannot evict the generation slot. Two slots roughly double KV memory — keep 1 on 4 GB GPUs.

Model: prithivMLmods/MiniCPM5-2B-GGUF revision 8b969e82c3ea123d604242f6d97a93b98a452070, MiniCPM5-2B.Q6_K.gguf. Size 2,070,227,904 bytes. SHA-256 d39e78a06dbb9b28ed9a9118b1370e992caf862003c925264ea33789b3e416bc, obtained from Hugging Face LFS metadata. This is a community quantization of OpenBMB's model.

`powershell -ExecutionPolicy Bypass -File scripts/prepare-runtime.ps1 -IncludeModel` downloads resumably and verifies artifacts before extraction/use. Assets stay in ignored .local/. Runtime validation is not yet complete.

## Ternary Bonsai 8B Q2_0

The catalog pins the original group-128 Q2_0 file (SHA-256 `3c8d70470a5d97e5a2b9410ddd899cb740116591462626c60cb2fead6448f60b`). Upstream b10855 interprets its tensor type differently and reports `output_norm.weight` offset 165015872, expected 174722688. A matching checksum means downloading this file again will not fix that error. New Prism releases also use the official group-64 Q2_0 layout and a separate PQ2_0 type for group-128 weights.

Use [Prism prism-b9601-68faa14](https://github.com/PrismML-Eng/llama.cpp/releases/tag/prism-b9601-68faa14) for this exact legacy model. Run `powershell -ExecutionPolicy Bypass -File scripts/prepare-runtime.ps1 -Bonsai` to install its pinned Windows CUDA 12.4 archive and matching CUDA DLLs into `.local/runtime-prism-b9601-68faa14`. Add `-IncludeModel` only if a model download is needed. Existing managed models in the app data folder can be selected directly.

Select that folder's `llama-server.exe`, select the verified Bonsai GGUF, save settings, and set Runtime context to 65,536 for the model's full context window. Lower it if available memory cannot accommodate the larger KV cache. The app probes runtime help before using `--no-agent`, which this older Prism release does not support; it still removes inherited `LLAMA_*` options and leaves built-in tools disabled. `node scripts/bonsai-runtime-smoke.mjs` verifies actual GPU offload and an arithmetic response with the same launch settings.

Verified on 2026-09-11 with an RTX 2050 (4 GB), driver 592.82, context 4,096, all GPU layers, Flash Attention and Q8 KV cache: 37/37 layers offloaded, successful response `42` to `17 + 25` both through the standalone server and LocalLM's native chat. GPU memory after app loading was approximately 2,341 MiB. Native settings are saved with the compatible runtime and the original verified model. The runtime capability check passed against both installed versions; the Rust suite passed 161 tests (two environment-dependent tests initially ignored), and the focused frontend suite passed five tests.

Follow-up verification on 2026-09-11 increased the saved and loaded context to 8,192 after another GPU inference test passed (37/37 layers; approximately 2,665 MiB GPU memory after a short chat). MCP selection now includes only requested live connector tools. Saved disconnected connectors are omitted for the reply, with a visible notice and a model-visible explanation; selections remain saved. `scripts/context-tools-smoke.mjs` verified a native request with 4,647 input + 512 reserved response tokens and five disconnected selections, returning `42` without a blocking error. Context preflight remains enforced; automatic compaction is enabled at 80% of the loaded context and can be disabled per conversation. Further work is planned in [ZAYA1 and FreeToken plan](ZAYA1-FREETOKEN-PLAN.md).

## Ternary Bonsai 2 27B PTQ1_0

Ternary Bonsai 2 stores rotated ternary g128 weights as `PTQ1_0` (or `PQ2_0`).
Those ggml type ids sit past upstream `GGML_TYPE_COUNT`, so stock llama.cpp
b10855 refuses the file (`tensor 'output.weight' has invalid ggml type 143`).
The activation-side Walsh–Hadamard transform is not upstream yet, so every
backend needs the [PrismML llama.cpp fork](https://github.com/PrismML-Eng/llama.cpp)
at **prism-b10658 or newer**. The legacy `prism-b9601` runtime used by Bonsai
8B Q2_0 cannot load these files either.

Install [prism-b10709-9a9394a](https://github.com/PrismML-Eng/llama.cpp/releases/tag/prism-b10709-9a9394a) with:

```
powershell -ExecutionPolicy Bypass -File scripts/prepare-runtime.ps1 -Bonsai2
```

That extracts the Windows CUDA 12.4 runtime and matching CUDA DLLs into
`.local/runtime-prism-b10709-9a9394a`. Clicking **Use model** on a `PTQ1_0` or
Bonsai-2 `PQ2_0` GGUF selects that `llama-server.exe`, applies a 4,096-token
first-load profile (automatic GPU fit, Flash Attention, Q8 KV, mmap, no vision
projector), and raises the startup poll window to 10 minutes for the 5.5 GiB
weights. Attach `Ternary-Bonsai-2-27B-mmproj-Q8_0.gguf` later from Model files
if you want image input; leave it off on a 4 GB GPU. `node scripts/bonsai2-runtime-smoke.mjs`
loads the managed PTQ1_0 file against this runtime.

Do not point stock llama.cpp, Ollama, or the b10855 CUDA installer at these
files. `PQ2_0` / `PTQ1_0` fail closed; a Bonsai 2 `Q2_0` file would load on
mainline and emit garbage.

## Switching between catalog models

The Models page saves the selected GGUF path immediately when its verified file
is present. Saving a Runtime context while that model is loaded reloads it;
clicking Use on the already-selected model keeps the saved context instead of
resetting to the first-load 8,192-token default. Loading then resolves the matching runtime profile: ZAYA uses its
custom runtime, Bonsai 8B Q2_0 uses the Prism `prism-b9601-68faa14` build,
Ternary Bonsai 2 uses `prism-b10709-9a9394a`, and MiniCPM
uses standard llama.cpp b10855. In development, the special runtimes are
found under `.local`; packaged installs keep the standard runtime in the
app-managed `runtimes` directory. A legacy saved Bonsai profile at 8,192, or
any value above 65,536, is clamped to the model's 65,536-token maximum before
launch; deliberately smaller values remain available for memory-constrained
systems. The runtime status
keeps the settings path spelling, so Windows' `\\?\\` canonical prefix cannot
make a successfully loaded model appear to need another switch.

## Actual layer offload reporting

The app starts the runtime with log verbosity 4 and, after successful model startup, reads at most the first 2 MiB of that load's log. It parses the runtime's `load_tensors: offloaded N/M layers to GPU` summary. Missing, malformed or inconsistent counts remain unavailable; requested GPU layers are never substituted for measured counts. Stopping or losing the runtime clears the report.

Runtime stdout and stderr are drained through a shared bounded writer. Each model load starts a fresh `runtime.log`, capped at 8 MiB including a truncation notice. After the cap, output continues to be drained and discarded so log backpressure cannot stop inference. File-write failures also keep draining the pipes. Startup diagnostics are retained; diagnostics emitted after the cap are unavailable until the next model load. Shutdown gives drain tasks a bounded opportunity to finish after stopping the child. This is bounded prefix retention, not rotating log history.

The pinned upstream [model loader](https://raw.githubusercontent.com/ggml-org/llama.cpp/7d701b592/src/llama-model.cpp) emits this summary, including its output-layer accounting. The [logging callback](https://raw.githubusercontent.com/ggml-org/llama.cpp/7d701b592/common/log.cpp) maps library information messages to trace verbosity, explaining why the default level 3 does not include the summary. Level 5 debug logging is not enabled. Trace logs contain additional model, memory and sampler metadata and should be treated as local diagnostic data.

`scripts/offload-smoke.mjs` verified the real MiniCPM runtime and UI for requests of -1, 5 and 0 layers. Reported counts were 43/43, 5/43 and 0/43 respectively. The original saved configuration and loaded/stopped state are restored afterward. This is model-layer placement evidence, not GPU utilization or proof that every inference operation executes on the GPU. Device-specific allocation and CPU/GPU cache placement reporting remain to be implemented. The earlier release installer predates this change.
