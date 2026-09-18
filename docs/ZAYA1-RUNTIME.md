# ZAYA1-8B local runtime

The standard runtime does not implement the `zaya` architecture. The pinned
community GGUF works with the experimental implementation from
[llama.cpp PR #23112](https://github.com/ggml-org/llama.cpp/pull/23112).

## Rebuild and verify

From the project directory, run:

```powershell
./scripts/prepare-zaya-runtime.ps1
node scripts/zaya-runtime-smoke.mjs
```

The build script requires Visual Studio 2022 C++ Build Tools and its CMake
component. It downloads and checks the source archive, then builds a separate
CPU runtime without replacing the MiniCPM or Bonsai runtimes. No CUDA toolkit
is required. Keep the DLLs alongside the generated executable.

- Source: `Juste-Leo2/llama.cpp`, revision `3750f9ce7ac20f7a905b43d9f20ad1050884f6c7`.
- Source archive SHA-256: `4450a6b172e53001e6b16708912e547d78b637d4095b5f8c6b592ce3ae17781d`.
- Executable: `.local/llama.cpp-3750f9ce7ac20f7a905b43d9f20ad1050884f6c7/build/bin/Release/llama-server.exe`.
- Model: the app-managed `ZAYA1-8B-Q4_K_M.gguf`, 5,567,581,549 bytes (5.19 GiB).
- Model SHA-256: `330ad2b15a6dabc9d955e7f10f4f7ee220180f06ee3bee4d06062218966f2c74`.

## App settings

Select the generated executable and verified ZAYA GGUF under **Models & runtime**.
Save these Runtime settings before loading:

| Setting | Value |
| --- | --- |
| Context | 8,192 |
| GPU layers | 0 (CPU) |
| CPU threads | 6 |
| Batch / micro batch | 128 / 32 |
| Flash Attention | Off |
| K / V caches | f16 / f16 |
| Offload KV cache | Off |
| Memory mapping | On |

In **Generation**, the app migrates ZAYA toward **8,192 maximum response tokens**,
then caps that reserve so it cannot fill the whole loaded context. At the
recommended 8,192-token window the reserve becomes 4,096, which leaves room
for a first prompt. Saving a larger Context window and reloading the model
raises the cap; the previous 8,192/8,192 pairing rejected every message.
The custom runtime also receives a separate **2,048-token reasoning cap**, so
thinking cannot consume the entire answer budget. Increasing the response
setting takes effect on the next response without reloading the model;
changing context still requires a reload, which Save configuration now
triggers when a model is already loaded.

The model uses system RAM in this configuration. Its weights exceed 4 GiB
VRAM; a GPU configuration would need separate build and offload verification.
Use the Chat preset for an initial conversation. Local ZAYA turns currently
disable tool definitions because this experimental runtime can emit tool-call
prose instead of structured calls; this prevents the model from reasoning until
the response limit while an unfinished call is never executed. Tool use needs a
separate runtime/parser verification before it can be enabled safely.

## Verification on 2026-09-12

The original standard runtime failed with `unknown model architecture: 'zaya'`.
The custom CPU build loaded this exact checkpoint at 8,192 context, passed
`/v1/chat/completions/input_tokens`, and answered `42` to `17 + 25` with a normal
stop. The Ryzen 5 7535HS generated 75 tokens at approximately 21 tokens/second;
the complete cold-start test took 21 seconds. This is one smoke measurement,
not a general performance benchmark.

`scripts/zaya-app-smoke.mjs` verifies native loading, streaming chat, persisted
messages, and the visible answer after reload. It requires the development app
at port 1420 with WebView CDP at port 9223. It backs up model preferences and
runtime settings under `.local/zaya-settings-before-<timestamp>.json`, restores
them on failure, and retains the working ZAYA configuration on success.
Reports are written under `test-results/zaya-*-smoke.json`.

## Response-limit fix evidence

The previous 2,048-token setting allowed a weather request to spend its entire
budget planning an unavailable web-search call. The app now migrates ZAYA to an
8,192 response reserve, caps reasoning at 2,048 runtime tokens, and omits tool
schemas for local ZAYA turns. On 2026-09-13 the same weather prompt completed
with a clear answer explaining that no weather tool was available; it did not
hit the response-limit error. Evidence is in
`test-results/zaya-response-budget.json`.
