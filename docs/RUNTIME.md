# Runtime provenance

Development runtime: ggml-org/llama.cpp release b10855, Windows x64 CUDA 12.4. Archives and SHA-256 digests are pinned in scripts/prepare-runtime.ps1 using GitHub release asset metadata.

llama-server is launched with `--parallel` from Runtime **Inference slots** (default 1). Local chat completions send `cache_prompt: true` and `id_slot: 0` for the parent conversation; subagents use `id_slot: 1` when two slots are enabled. Token counting (`/v1/chat/completions/input_tokens`) omits both fields so preflight cannot evict the generation slot. Two slots roughly double KV memory — keep 1 on 4 GB GPUs.

Model: prithivMLmods/MiniCPM5-2B-GGUF revision 8b969e82c3ea123d604242f6d97a93b98a452070, MiniCPM5-2B.Q6_K.gguf. Size 2,070,227,904 bytes. SHA-256 d39e78a06dbb9b28ed9a9118b1370e992caf862003c925264ea33789b3e416bc, obtained from Hugging Face LFS metadata. This is a community quantization of OpenBMB's model.

`powershell -ExecutionPolicy Bypass -File scripts/prepare-runtime.ps1 -IncludeModel` downloads resumably and verifies artifacts before extraction/use. Assets stay in ignored .local/. Runtime validation is not yet complete.

## Switching between models

The Models page saves the selected GGUF path immediately when its verified file
is present. Saving a Runtime context while that model is loaded reloads it;
clicking Use on the already-selected model keeps the saved context instead of
resetting to the first-load default. Loading falls back to the standard
llama.cpp b10855 runtime when no executable is selected; a user-chosen
`llama-server.exe` is left untouched. In development the runtime lives under
`.local/runtime`; packaged installs keep it in the app-managed `runtimes`
directory. A saved context above the model's GGUF-advertised maximum is refused
before launch. The runtime status
keeps the settings path spelling, so Windows' `\\?\\` canonical prefix cannot
make a successfully loaded model appear to need another switch.

## Actual layer offload reporting

The app starts the runtime with log verbosity 4 and, after successful model startup, reads at most the first 2 MiB of that load's log. It parses the runtime's `load_tensors: offloaded N/M layers to GPU` summary. Missing, malformed or inconsistent counts remain unavailable; requested GPU layers are never substituted for measured counts. Stopping or losing the runtime clears the report.

Runtime stdout and stderr are drained through a shared bounded writer. Each model load starts a fresh `runtime.log`, capped at 8 MiB including a truncation notice. After the cap, output continues to be drained and discarded so log backpressure cannot stop inference. File-write failures also keep draining the pipes. Startup diagnostics are retained; diagnostics emitted after the cap are unavailable until the next model load. Shutdown gives drain tasks a bounded opportunity to finish after stopping the child. This is bounded prefix retention, not rotating log history.

The pinned upstream [model loader](https://raw.githubusercontent.com/ggml-org/llama.cpp/7d701b592/src/llama-model.cpp) emits this summary, including its output-layer accounting. The [logging callback](https://raw.githubusercontent.com/ggml-org/llama.cpp/7d701b592/common/log.cpp) maps library information messages to trace verbosity, explaining why the default level 3 does not include the summary. Level 5 debug logging is not enabled. Trace logs contain additional model, memory and sampler metadata and should be treated as local diagnostic data.

`scripts/offload-smoke.mjs` verified the real MiniCPM runtime and UI for requests of -1, 5 and 0 layers. Reported counts were 43/43, 5/43 and 0/43 respectively. The original saved configuration and loaded/stopped state are restored afterward. This is model-layer placement evidence, not GPU utilization or proof that every inference operation executes on the GPU. Device-specific allocation and CPU/GPU cache placement reporting remain to be implemented. The earlier release installer predates this change.
