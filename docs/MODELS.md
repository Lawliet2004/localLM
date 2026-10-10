# Local model library

Open **Models & runtime → Models** for an LM Studio-style workspace: **Discover**
to browse Hugging Face, **My Models** to load or delete what you already have.
The bundled catalog model appears in My Models as a Hugging Face entry.
The chat picker also lists the downloaded library.

## Download, use, delete

1. Discover loads up to 30 popular GGUF repositories on open. Search by model or
   publisher, paste an `owner/repository`, or drop a Hugging Face URL.
2. The right pane lists every GGUF quantization with size and a **Fits GPU** /
   **Fits RAM** / **May not fit** estimate for this machine. Split GGUF models
   download all shards from the same immutable revision together. Q4_K_M is
   preselected when present.
3. Download. The harness checks disk space and verifies every file's SHA-256
   before publishing the completed model. Progress and cancellation remain
   available while downloading. If the transfer is cancelled, stalled, or the
   app closes, the unfinished `{name}.gguf.part` file and a `download.json`
   sidecar stay on disk. **My Models** shows how many bytes were saved and
   **Resume download** continues with HTTP Range instead of starting over.
   A finished file that fails SHA-256 is discarded. Empty zero-byte parts are
   removed.
4. Choose **Use model**. The harness saves the outgoing model's runtime settings,
   restores the incoming model's settings, loads it, and selects local inference.
5. Choose **Delete**, then **Delete from disk**, to remove its files. An active
   copy is unloaded first. Conversations are retained. Deletion also removes the
   managed `model.json` manifest, an empty hash directory, and the saved runtime
   profile. Matching catalog copies (app-data and `.local/models`) are grouped
   and removed together.

Search returns up to 30 popular matches with download counts; enter an exact
repository to browse anything else. For gated/private repositories, accept the
license on Hugging Face and enter a read token in the access section. Tokens stay
in page memory and are not persisted in model manifests, preferences, or logs.

## Compatibility

Downloads are not restricted to a curated catalog. Any repository's GGUF weight
files with Hugging Face LFS size/checksum metadata can be selected. Inference
requires an architecture and quantization supported by the selected llama.cpp
build. Safetensors/PyTorch weights require external conversion. Projectors,
adapters, embedding-only models, and other companion files are not standalone
chat models. The harness does not execute downloaded model code.

Models use the standard llama.cpp runtime when no executable is selected. On
machines with the neighboring K2 Horizon project and its downloaded GGUF, the
model appears in **My Models** and the chat model picker. Selecting it uses
the custom CUDA llama-server from the
neighboring `k2 horizon agent` project. Selecting another model without a saved
runtime profile returns to the standard runtime. The K2 model remains owned by
its original project, so LocalLM cannot delete it. **Model files** retains
manual model and runtime paths, runtime installation, and the original pinned
download controls.

## Runtime guidance

Context length is capacity **per inference slot**. One slot emits `--ctx-size N --parallel 1`; two emit `--ctx-size 2*N --parallel 2`. KV estimates include both slots. Model-limit validation compares N to the trained maximum. After loading, authenticated `/props` (`default_generation_settings.n_ctx`) supplies measured capacity; a missing or invalid value falls back to N. Nested worker calls use slot 1 with a managed two-slot runtime, while chat uses slot 0. Calls within a worker queue serially; one-slot and external loopback providers leave the slot unspecified.

The Runtime page reads architecture, context capacity, transformer blocks,
attention dimensions, and weight size from the GGUF header. Split-model sizes
include all shards. GPU offload shows two maxima: the model’s layer count
(transformer blocks + output) and how many layers are estimated to fit in
currently free VRAM. The full model context and the harness safety cap (2,097,152
tokens) are shown separately; saving or loading refuses a context beyond known
model limits. GPU controls explain all-layer, split, and CPU placement. Runtime
telemetry reports the actual loaded offload count next to the requested value.

Live estimates show weight/cache placement in VRAM and RAM as settings change.
Cache estimates are available for supported standard-attention layouts. Hybrid,
recurrent, and other unrecognized layouts explicitly show unknown cache costs.
Estimates exclude compute buffers, runtime overhead, and memory-mapped pages;
they are planning aids, not measured allocation or a guarantee that a model fits.

Recommendations use an initial context of at most 8K and the available hardware
snapshot, with roughly 1 GiB of GPU headroom. They suggest CPU placement when
telemetry or cache layout is unknown. Quantized-cache recommendations check head
alignment. First-time loads use F16 caches with Flash Attention disabled for
broader runtime compatibility; saved model-specific choices take precedence.

## Storage and verification

- New downloads: app data `models/<hash-of-repo-revision-and-file>/`.
- Provenance/completion: `model.json` in that directory; unpublished or incomplete
  shard groups cannot be selected for inference.
- Runtime profiles: app data `model-profiles/<hash-of-model-path>.json`.
- Existing managed files and development `.local/models` files are discovered
  in place. Matching pinned development copies are grouped, and deletion removes
  all displayed locations. Files outside the inventory cannot be deleted by ID.
- Network metadata is bounded; repository/file inputs reject path traversal.
  Downloads use HTTPS and reject destination links outside managed storage.

Implementation follows the [Hugging Face Hub API](https://huggingface.co/docs/hub/api)
and [GGUF metadata specification](https://github.com/ggml-org/ggml/blob/master/docs/gguf.md).

## Qualification and readiness

A model that loads and answers one prompt is not qualified. **Run qualification**
in Models sends a fixed, deterministic suite (temperature 0, small output caps,
per-case timeouts) and records each case as passed, failed, unsupported, or
untested — never as a load-and-hope check:

- `basic_chat` — a fixed question; requires a non-empty streamed answer.
- `structured_output` — a small JSON schema must be returned intact.
- `tool_call` / `tool_args_parsing` — a function call with typed, nested
  arguments must round-trip through the same parser the turn loop uses.
- `malformed_recovery` — a corrupt tool result gets one retry.
- `simple_coding` — a generated function is run locally when a Python runtime
  is available; otherwise `untested` rather than a silent pass.
- `grounded_research` — only tested when web search is configured.
- `cancellation` — a long generation must stop within 5 seconds and leave the
  model responsive.

Overall `passed` requires every core case (chat, both tool-call cases,
structured output) to pass; a failure in any of them yields `partial` or
`failed`, and a cancelled run is `cancelled`. Results persist per
**model identity** — model file, runtime version, provider, and generation
config — hashed into `identity_hash`. Change the model or its config and the
saved result is shown as stale until re-run.

The single **readiness** indicator walks the setup chain in order — runtime,
model selection, load state (including OOM hints for low-VRAM machines),
provider reachability, then qualification — and names the one next action.
Chat shows this card until the setup is actually ready.

**Profiles** (Chat / Coding / Research / Advanced, in Settings → Tools) map to
the existing tool presets and interaction modes. Recommendations come from the
qualification record: a model that cannot call tools recommends only Chat and
explains why under Coding and Research.
