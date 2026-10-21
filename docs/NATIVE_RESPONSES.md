# Native Responses provider adapter

Select **OpenAI Responses** in a provider connection's API format. Supply the
base API URL (for example, `https://api.openai.com/v1`); the native backend sends
requests to `/responses`. This choice is explicit and uses the existing endpoint
validation, credential binding, local-only policy, cancellation and bounded
pre-stream retry path. It adds no dependencies.

`responses_adapter.rs` converts the harness's canonical messages, embedded image
inputs, client-owned function definitions, named tool choice and output schema
into native Responses fields. Conversion returns an error before network I/O
for invalid input. `store:false` is always set. The adapter enables client
function tools; it rejects server-owned tools and unsupported stream events.

The stream adapter distinguishes a function's `call_id` from its output-item
`id`, maps interleaved native output indexes to dense tool-call indexes, and
reconciles argument/text deltas with final items without duplicating them.
`response.completed` supplies the terminal output and usage without requiring
`[DONE]`. Token exhaustion cannot authorize incomplete tool calls. Refusals,
provider failures and malformed streams remain explicit. Cached input tokens
are included in input usage rather than added twice.

Opaque native output items, including encrypted reasoning context and message
phase, are replayed in their original form. Durable tool-round metadata is
anchored before ordinary execution and before parallel read dispatch. Read
settlement reuses the reserved audit row. Restart and fork replay retain this
metadata; editing a completed tool audit invalidates it. A missing durable
result produces an explicit unknown-outcome result and forbids automatic
repetition. Private native replay fields are removed when using a different
provider format.

Remaining lifecycle paths are covered as a draft: continuation, web-lookup
retry, plan and tool-call-repair rounds preserve native output items instead of
flattening to text; each round resets its replay answer so a stale round cannot
overwrite the final answer; finishing rounds anchor their native context on the
finish audit row and record suppressed same-round calls as explicit unknown
outcomes; text-parsed literal calls strip provider-native fields and replay
canonically rather than fabricating native items; and audit groups that mix
model calls with program (plan-*/harness) calls degrade to canonical replay
instead of failing history reconstruction. A native-context persistence failure
settles the audit visibly and prevents tool dispatch.

The existing known-model reasoning mapping also applies to native Responses:
Quick/Think maps to low/high effort for the existing recognized families and
omits incompatible sampling fields. Explicit native reasoning takes precedence.
This remains a legacy family heuristic, **not** verified support. Explicit
[per-model controls](MODEL_CAPABILITIES.md) now replace that mapping when
configured, preserve Unknown support and enable constrained child generation.
Unknown models retain the provider's defaults. Remote context preflight is a
character estimate with a bounded image reservation, not an exact tokenizer.

Bounds include 64 advertised tools, 128 native output items, eight function
calls, 64 KiB per function's arguments, 4 MiB cumulative native stream data and
16 MiB serialized requests. The shared SSE decoder also enforces its event
bound. Unsupported or excessive input fails visibly rather than truncating.

## Verification and remaining acceptance

Offline coverage exercises conversion, schema/boolean errors, interleaved
stream identity, completion-only output, usage, refusal/truncation, malformed
streams, cross-format metadata removal, native reasoning controls, SQLite
restart/fork/edit behavior, cancelled parallel-read settlement, finish-round
anchoring with suppressed-call unknown outcomes and mixed program+model
canonical degradation (both verified end-to-end through `responses_adapter`
request conversion). The provider editor has save/test-format coverage. Shared
parent and child stream collectors use the adapter selected by the backend.

Unignored loopback HTTP fixtures cover a complete parent tool round and child
stream/replay exchanges. The current coordinator sandbox denies TCP listener
binding; these fixtures require execution in an allowed native test environment.
No live external provider or paid API request has been made. Fresh provider UI
captures and final independent SWE-2 Max review remain pending.

The capability/constrained-child implementation (A12) is now a draft with
offline coverage; its host HTTP execution and final acceptance remain open.
The native no-tool retry/fallback history lifecycles (X04) are implemented as a
draft per the paragraph above; the final merged acceptance audit and
independent review remain.

Protocol references: [function calling](https://developers.openai.com/api/docs/guides/function-calling),
[streaming](https://developers.openai.com/api/docs/guides/streaming-responses),
[reasoning and stateless replay](https://developers.openai.com/api/docs/guides/reasoning),
and [structured outputs](https://developers.openai.com/api/docs/guides/structured-outputs?api-mode=responses).
