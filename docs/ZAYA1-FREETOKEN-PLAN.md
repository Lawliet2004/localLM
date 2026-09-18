# Plan: improve LocalLM with ZAYA1-8B and evaluate FreeToken

**Local runtime update (2026-09-12):** ZAYA1 now runs on this laptop using a
separate CPU build of the pinned experimental runtime. Native chat, context
preflight, and persisted output after reload passed. See
[ZAYA1 runtime setup and evidence](ZAYA1-RUNTIME.md) for the working configuration.
The compatibility-blocker notes below describe the earlier investigation;
GPU offload and the broader model comparison remain unverified.

Prepared 2026-09-11. Status: Step 1 (Context & Tool Costs) implemented and verified; Step 3 adapter preparation (loopback optional auth, FreeToken format routing, mock-provider tests) implemented; subsequent model evaluation steps pending hardware/environment gating. Verified 2026-09-12: `cargo test --lib` 172 passed, `npm test` 83 passed, `npm run build` clean. Native hardware smokes (`bonsai-runtime-smoke.mjs`, `context-tools-smoke.mjs`) still require the running desktop app with a loaded model.

## Goal

Improve reasoning, coding and longer tool-assisted conversations on this Windows laptop without losing the working Bonsai setup. Adopt a new model or runtime only after it improves measured local tasks.

## Current baseline

- RTX 2050: 4,096 MiB VRAM, compute capability 8.6; Ryzen 5 7535HS; approximately 15.2 GiB usable system RAM. Only about 2.4 GiB system RAM was free during inspection.
- Bonsai Q2_0, Prism b9601, Q8 KV cache, Flash Attention, 8,192 context, all 37 layers on GPU. Approximately 2,665 MiB GPU memory used after a short native chat.
- Fixed this session: connected MCP servers no longer automatically contribute unselected tools. Disconnected saved connector choices are omitted for that reply with a visible notice and a model-visible explanation. Choices and authorization settings remain saved.
- Native regression passed with 4,647 input tokens + 512 response reserve, including five disconnected saved tools. Response: `42`; no blocking error.
- Context is still finite. At plan time `src-tauri/src/compaction.rs` was a stub: auto-compaction always returned false and manual compaction was not implemented (resolved by Step 1 above). The Standard preset still injects numerous harness tool schemas. Increasing context alone does not solve arbitrary conversation growth.

## Findings and recommendation

**Keep Bonsai as the default; evaluate ZAYA1 as an optional reasoning specialist.** Zyphra describes ZAYA1 as a reasoning-focused MoE with 760M active and 8.4B total parameters. Its published results emphasize math and coding, while tool-use results do not establish it as a universal agent upgrade. Those are publisher evaluations, not measurements on this laptop. [Model card](https://huggingface.co/Zyphra/ZAYA1-8B).

Active parameter count describes computation, not the total weight storage. Approximate raw weight arithmetic for 8.4B parameters is 16.8 GB at BF16 or 4.2 GB at four bits, before scales, unquantized layers, caches and runtime allocations. Four-bit ZAYA1 therefore cannot be assumed to fit entirely in 4 GB VRAM. A supported CPU/GPU split needs measurement; BF16 is not a sensible first local trial given current free RAM.

**FreeToken is a separate runtime experiment.** It offers expert offload/caching and CPU/GPU cooperation. Those MoE features do not directly accelerate the dense Bonsai model. Its public GPU support list names RTX 30/40/50 series; this laptop reports SM 8.6, so kernel compatibility needs an actual check rather than inferring support from the product name. [FreeToken overview](https://github.com/FlashML-org/FreeToken).

At inspected revision `0ffd5c8b2941974ed64dec09170b19259e2ba5aa`, FreeToken's registry does not contain ZAYA1's `ZayaForCausalLM` architecture. Changing a model ID or using an OpenAI-compatible client cannot supply the missing engine implementation. Combining them would require upstream support or a substantial model port. [Pinned registry](https://github.com/FlashML-org/FreeToken/blob/0ffd5c8b2941974ed64dec09170b19259e2ba5aa/python/freetoken/models/register.py).

## Ordered implementation steps

### 1. Make context and tool costs visible — first priority, medium effort [COMPLETED 2026-09-11]

1. Add a debounced preflight count for the current draft, using the same rendered payload as send. Show history, instructions, tool schemas and response reserve separately; distinguish exact counts from provider estimates.
2. Add explicit Chat, Research and Coding tool profiles. Chat should not automatically receive the full Standard harness. Research exposes only selected live search/fetch tools; repository documentation tools are opt-in. Preserve existing choices when switching conversations.
3. Implement compaction as a bounded checkpoint plus recent complete turns. Preserve system instructions, current user input and matched tool-call/result groups. Keep full transcripts in SQLite; do not silently delete history. Count again after compaction and explain cases where the current message/tool catalog alone exceeds capacity.
4. Make tool-result excerpts depend on remaining token budget. Reuse the existing artifact store and `artifact_read` for full results instead of embedding large pages repeatedly. Avoid aggressive rewriting of tool JSON schemas.

Touch points: `src-tauri/src/chat.rs`, `context.rs`, `compaction.rs`, `history.rs`, `artifacts.rs`, `presets.rs`; `src/components/Chat.tsx`, `ToolControls.tsx`, `src/lib/api.ts`, `types.ts`.

Acceptance: short questions work with offline saved connectors; a long chat can compact and continue without losing a required fact; oversized single messages remain recoverable as drafts; tool selection stays explicit.

### 2. Establish a ZAYA1 reference before optimizing — medium effort, compatibility gate [BLOCKERS VERIFIED 2026-09-12; two unblock paths documented]

1. Pin the model revision and an engine revision that implements ZAYA1. Current Transformers main contains a ZAYA implementation; the model card also documents Zyphra's vLLM fork. Validate the chosen release rather than blindly following older branch-install commands. [Transformers implementation](https://github.com/huggingface/transformers/blob/main/src/transformers/models/zaya/modeling_zaya.py).
2. Validate tokenizer/chat template, stop tokens, reasoning output, quantization support and actual memory requirements before any large download. Do not treat a GGUF filename as proof the installed Prism runtime supports the architecture.
3. If a supported quantized checkpoint and CPU-offload path fit available RAM, run an isolated trial with one model loaded, short context and one request at a time. If not, document the memory blocker; use a separately approved compatible host for a reference result.
4. Compare the same 30 prompts against Bonsai: 10 coding tasks checked by tests, 10 math tasks with known answers, five instruction-following tasks and five tool-use tasks. Measure correctness, first-token latency, output rate, total reasoning tokens, peak RAM/VRAM, cancellation and errors. Use repeated runs for latency.

Acceptance: choose ZAYA1 for a category only if it improves task success without unacceptable latency, memory pressure or tool failures. Larger reasoning budgets are an opt-in profile, not the default for every chat.

**Verified blocker status (2026-09-12):**

- Zyphra publishes only BF16 safetensors (~17.7 GiB, 4 shards). No official GGUF exists; the catalog previously pointed at a nonexistent official Q4_K_M file with placeholder metadata — it has been repointed to the real community checkpoint (see below).
- llama.cpp: the feature request was closed stale, and the draft implementation ([PR #23112](https://github.com/ggml-org/llama.cpp/pull/23112)) was closed unmerged; `zaya` is absent from `llama-arch.cpp` on master. The community quants at [Abiray/ZAYA1-8B-GGUF](https://huggingface.co/Abiray/ZAYA1-8B-GGUF) were produced with that draft branch and load only in a self-built runtime from it.
- `arxyzan/zaya-1b-it` is a third-party Gemma3-architecture 1B model, not ZAYA1-8B; do not substitute it into the comparison.

**What the app now does about it:**

- The managed catalog pins the community `ZAYA1-8B-Q4_K_M.gguf` to revision `e16067cfd1f73cc688ec4004573f33de76aa88bf` (5,567,581,549 bytes, sha256 `330ad2b15a6dabc9d955e7f10f4f7ee220180f06ee3bee4d06062218966f2c74`), so an integrity-verified download is possible.
- `gguf::read_architecture` checks `general.architecture` at load time; a `zaya` checkpoint is refused for the managed Prism runtime with an actionable message. A user-selected custom runtime path is allowed through, so a PR-#23112 build works once available.
- The Models page download panel carries the runtime and memory advisory for this entry (5.6 GiB weights cannot fit 4 GiB VRAM; partial offload needs ~3 GiB free system RAM and a small context).

**Unblock paths:**

1. *Patched llama.cpp runtime*: build `llama-server` from llama.cpp PR #23112, select it in Models & runtime, then download the pinned checkpoint — the existing catalog entry then works end-to-end (subject to the memory advisory).
2. *External engine*: serve ZAYA1 with an engine that implements the architecture (Transformers main, or Zyphra's vLLM fork) behind an OpenAI-compatible loopback endpoint, and connect it under Providers — verified loopback engines work without an API key. On this laptop the memory gate still applies (~6+ GiB free RAM for a 4-bit quant); otherwise use a separately approved host for the step-2 reference benchmark.

### 3. Connect an external engine through the existing provider adapter — small/medium effort [ADAPTER PREP DONE 2026-09-12; engine connection still gated on a running engine]

Reuse `src-tauri/src/providers.rs`, `inference.rs` and `src/components/ProviderManager.tsx`; do not replace the Tauri application or merge Python inference into its process.

FreeToken documents `/v1/models` and streaming `/v1/chat/completions`, normally at `http://127.0.0.1:1919`. Discover the served model ID and configure the actual server context capacity. Verify token-count endpoint support separately: LocalLM currently estimates remote-provider context. [API quickstart](https://github.com/FlashML-org/FreeToken/blob/main/docs/quickstart.md).

LocalLM currently requires an API key and `hasApiKey` for provider readiness. Add explicit optional authentication for verified loopback-only engines if needed, while retaining HTTPS and credentials for remote hosts. Test streaming, reasoning separation, tool-call JSON, cancellation, server loss and reconnection. Switching back to Bonsai must restore its runtime profile.

Implemented adapter preparation: verified loopback-only providers may omit an API key (`providers::is_loopback_base_url`); keyless requests send no Authorization header; a `freetoken-openai` provider format routes through the OpenAI chat-completions adapter with UI and readiness gating updated (`ProviderManager.tsx`, `App.tsx`); mock-provider Rust tests cover keyless loopback streaming and draft validation. Live verification against a running FreeToken server remains gated on step 4's environment.

### 4. Evaluate FreeToken independently — gated experiment, medium effort

1. Use a pinned, isolated Linux/WSL environment; check WSL availability, GPU visibility and the specific kernels on SM 8.6 before downloading weights. The CLI installation docs specify Linux x86_64, driver r580+, and CUDA 13 toolkit for JIT kernels; the Windows desktop wrapper is a different installation route. [Installation requirements](https://github.com/FlashML-org/FreeToken/blob/main/docs/install.md).
2. Check the exact engine dependency set. The inspected package uses Torch 2.11 and documents a Transformers-version conflict for an optional Marlin path. Do not install these packages into the working app environment. [Pinned dependencies](https://github.com/FlashML-org/FreeToken/blob/0ffd5c8b2941974ed64dec09170b19259e2ba5aa/pyproject.toml).
3. Select a supported checkpoint only after calculating total host-memory needs. Offload moves weights into RAM; it does not make a 20B/35B checkpoint fit the current free RAM automatically. Stop at a clear resource failure instead of relying on heavy swapping. [Supported models and offload modes](https://github.com/FlashML-org/FreeToken/blob/main/docs/models.md).
4. If a checkpoint fits, benchmark CPU, offload and hybrid modes with identical prompts, context and quantization. Calibrate using `ft bench bw`; record warm/cold cache performance and total task latency. [CLI reference](https://github.com/FlashML-org/FreeToken/blob/main/docs/cli.md).

Acceptance: adopt FreeToken only after a supported configuration measurably beats the reference and passes the same API/tool correctness tests. Cache reuse can reduce repeated computation; it does not enlarge the model's context limit or eliminate storage needs.

### 5. Combine ZAYA1 and FreeToken only after architecture support exists — large effort

Recheck the registry at the chosen release. If still absent, keep separate backends. A dedicated port would need configuration/weight mapping, MoE routing, ZAYA compressed convolutional attention and recurrent states, cache lifecycle, quantization, and tokenizer/reasoning/tool parsing tests against a reference engine. This is not the recommended first investment for a 4 GB GPU.

## Rollout and verification

Land context improvements first, then add one optional provider and evaluate one model at a time. Store model/runtime revisions, settings and benchmark outputs so results can be reproduced. Keep the working Bonsai profile available throughout.

Existing checks: `cargo test --manifest-path src-tauri/Cargo.toml --lib`, `npm test`, `npm run build`. Native hardware checks: `scripts/bonsai-runtime-smoke.mjs` with an explicit context argument, and `scripts/context-tools-smoke.mjs` against a running 8,192-context native app. Add mock-provider tests before external engine integration.

No ZAYA1 downloads, FreeToken installation, cloud usage or engine migration have been executed as part of this plan.
