# Prompt caching and tool protocols

The harness preserves model-visible history, requests cache usage from the
backend, and validates complete tool calls before permissioned execution.
These changes affect the Rust inference loop and context display; they do not
change model weights or silently select a remote provider.

## Cache behavior

- The UI draft and the rendered user content are stored separately, atomically
  with the accepted turn. Historical clock, memory, plan and resume injections
  replay unchanged instead of disappearing at the first historical user message.
- One model transcript is stored per conversation after a successful turn.
  It includes internal planning, continuation and tool-result messages, rather
  than reconstructing them from a consolidated display answer. It replaces the
  previous snapshot instead of keeping a growing copy for every turn.
- Snapshots require the same last recorded message and compaction checkpoint.
  Message edits and deletions invalidate them. Newly appended rows prevent reuse.
  A complete message-based fork copies a valid transcript; partial forks and
  older conversations use audited history and saved message content.
- Tool ordering and descriptions remain frozen for a selection. Changes to tool
  names or input schemas refresh the catalog so the model sees the live contract.
- Exact local tokenizer counts use a 256-entry memory cache with a five-minute
  TTL and eviction of the least recently used idle entry. Identical concurrent
  requests share one count. Errors are not cached, cancelled requests release
  their entry, and the runtime endpoint, authentication nonce and normalized payload
  form the SHA-256 key. The cache stores no prompt or credential text.
- Local inference retains `cache_prompt` and slot affinity, and requests streaming
  usage and timings. Native Claude requests use automatic ephemeral prompt
  caching. OpenAI-compatible requests keep standard streaming usage options and
  exclude llama.cpp-only controls.

The context popover displays reused input tokens, their fraction of the last
generation's input, cache writes and prompt-processing time when reported.
The session event stream also records `inference_usage` for each measured chat
round. Missing provider counters remain unknown; a missing counter is not a
claimed cache miss or hit. Anthropic input totals include uncached, cache-read
and cache-write tokens. Repeated cumulative usage events are merged, not summed.

## Tool protocols

Foreground chat and both child-agent drivers use the same stream normalizer.
SSE supports LF, CRLF and bare CR line endings, including mixed endings and
multibyte Unicode split across individual network bytes.
OpenAI-compatible streams finish with `[DONE]`; native Claude streams finish
with `message_stop`. A finish reason alone does not terminate reading, so
usage-only trailing events are retained. Missing completion markers, missing or
unsupported finish reasons, truncated JSON and token-limited tool generations
cannot authorize execution.

Claude content-block indices are mapped to dense tool indices: preceding text
or thinking blocks do not consume the eight-call budget. Empty tool inputs
become `{}` when their block closes; interleaved JSON fragments are assembled
before validation. Tool results from one parallel batch return in one user
message. The adapter preserves tool choice, explicit tool selection and
parallel-use controls, respects the configured output limit, and avoids sending
both sampling controls together.

Structured and text-fallback calls share limits: eight calls per generation,
128-byte identifiers/names and 65,536-byte JSON arguments. Arguments must be
objects, and call identifiers must be unique. Text function blocks require
closing tags and reject duplicate parameter names. Existing approval and
dispatch rules still apply; tool results and side effects are never memoized.

## Reading large research results

The model's stable system instructions describe live search only when a search
tool survives offline, preset and model-compatibility filtering. Unfamiliar
names or missing training data should trigger retrieval. A response that claims
live access is unavailable before trying any lookup gets one correction with
tools still available. This retry respects explicit no-browsing requests and
does not repeat failed or denied lookups. Session events record the complete
rendered system instructions and any `web_lookup_retry`.

Research tool output carries one `evidence` field, with source URLs and saved
session/document IDs. When it exceeds the per-result context budget, the harness
keeps structured evidence and bounded source references rather than a raw JSON
prefix. The complete result remains in a conversation-scoped artifact.

Every preset offers the trusted `artifact_read` tool so the model can recover
omitted evidence without repeating the search. Artifact pages retain their
original ID, character offset, returned limit and `hasMore` cursor even when a
page itself needs shortening. Continue using the original ID at `offset + limit`.
An excerpted result does not indicate a network failure or missing source data.

Regression tests cover benchmark data beyond the former 1,200-character preview,
source and document handles, Unicode byte limits, all preset inventories, and
complete artifact recovery at 512-, 2,048- and 8,192-byte result budgets.

### Live lookup check on 2026-10-16

The follow-up failure contained zero tool calls despite exposing `web_search`.
With explicit tool-use guidance, the same loaded Gemma E4B Q2 model selected
`web_search` automatically for the same benchmark-comparison request. The real
worker returned 20 results and fetched six pages. The targeted Rust suite passed
99 tests after adding capability guidance and the bounded refusal correction.

This establishes retrieval, not answer accuracy: subsequent live generations
omitted citations or misreported numeric benchmark values even though the saved
evidence contained those values. Extra wording, plain-text evidence and a
lower-temperature/no-thinking experiment did not establish a faithful benchmark
table. Sampling settings, model weights and the stored conversations were not
changed. These outputs must not be counted as successful benchmark comparisons.

## Verification on 2026-10-15

- Targeted Rust suite: **122 passed**, including history restart/fork/edit
  behavior, native Claude child streams, Unicode split across network bytes,
  eight parallel calls, empty arguments, completion boundaries, schema refresh,
  mixed SSE line endings, usage accounting, tokenizer isolation, bounded eviction, expiry, cancellation,
  failure recovery and request sharing across token-count/budget-check paths.
- The tokenizer fixture issues four identical preflights, including three
  concurrent calls, and permits exactly one HTTP request. All four receive the
  exact count. This measures request reuse, not inference speed or a live KV
  cache hit rate.
- Frontend suite at verification time: **371 passed, 1 skipped**. TypeScript
  checking and the production build passed; the build reports its existing
  large-chunk warning.
- The broader Rust run excluding download tests reported **318 passed,
  5 failed, 2 ignored**. Failures require Windows credential storage or a
  configured PowerShell interpreter. Download tests also exercise a
  Windows-only disk-space function; the initial full run was stopped after
  its stalled-header fixture waited on a connection never opened on Linux.
- Strict Clippy encounters existing unused/dead-code and style warnings in
  existing and concurrently edited code. Checking with those warning categories allowed
  passed; this is not a claim that strict repository-wide Clippy is green.
- Concurrent desktop-capture work subsequently added a Linux PipeWire build
  dependency. The host lacks its development libraries, so the final 122-test
  regression run was also repeated successfully in an isolated source copy
  using the original dependency manifest and lockfile. The shared checkout's
  unrelated dependency changes were preserved.

Run the relevant Rust coverage with:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib -- \
  chat::finish_tests claude_adapter::tests context::tests history::tests \
  inference::tests sse::tests store::tests subagents::tests tool_calls::tests
npm test
npm run typecheck
npm run build
```

Live inference cache ratios, latency and model tool-selection quality were not
benchmarked. They depend on the loaded model/template, runtime support,
available KV state, provider cache thresholds and workload. Runtime restarts,
compaction, selection changes and competing work in a single slot can require
reprocessing. SWE-2 Max was used through the separately installed Devin CLI for
cache-lifecycle implementation and focused correctness research. Its findings
led to centralizing count-request normalization so streaming controls cannot
split equivalent prompts into separate cache entries. It added deterministic
expiry, cancellation, failed-fetch and normalized-key regressions.
The review also led to bare-CR SSE support. Its concern about top-level Claude
`cache_control` was checked against the current official automatic-caching
documentation linked below; the supported top-level field was retained.

## Protocol references

- [llama.cpp server API](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)
  documents prefix KV reuse, streaming usage and prompt/generation timings.
- [Anthropic prompt caching](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
  describes top-level ephemeral `cache_control` and cache-token accounting.
- [Anthropic streaming](https://platform.claude.com/docs/en/build-with-claude/streaming)
  describes content-block indices, input JSON fragments, cumulative usage and
  the `message_stop` completion boundary.
