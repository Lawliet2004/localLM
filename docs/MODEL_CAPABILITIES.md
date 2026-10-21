# Per-model inference controls

In **API providers → Edit → Models → Inference controls**, configure a model's
temperature, top-p, reasoning-effort, tool-choice, parallel-tool-control and
structured-output support as **Unknown**, **Supported** or **Unsupported**.
Choose the output-token field when a compatible endpoint needs a specific one.
These controls are separate from the existing tool and image input settings.

Existing connections retain their automatic model-family mapping until controls
are configured. An explicit controls object replaces those heuristics: unknown
reasoning support does not invent an effort value, and unknown structured-output
support does not enable provider-side child constraints. Unknown sampling/tool
controls may still be attempted for compatibility; unknown is never recorded as
verified support. Unsupported sampling controls are omitted. A requested tool
policy that an explicitly unsupported control cannot enforce fails visibly.
Quick and Think map to low and high effort on configured supported models.

The backend now retains the complete tool-support state instead of collapsing
Unknown into a stored supported boolean. The permission to *attempt* tool use is
separate from that state. Parent context and child prompts include a bounded
capability report. Its saved tool setting is explicitly unverified; optional
controls distinguish user configuration, endpoint reports and legacy heuristics.
A successful connection/inference probe does not verify every optional feature.

## Discovery, persistence and provenance

Model discovery imports recognized `supported_parameters` metadata conservatively.
Malformed or excessive parameter lists do not establish unsupported features.
Generic response-format support alone does not establish strict schema support;
the endpoint must report structured-output support specifically. Reported optional controls remain
unverified and are pinned to the API URL, format and model ID. Reusing a report
for a different target fails before inference. Fetch models again or configure
explicit controls to use a different target.

The form clears reported optional controls when changing the endpoint, format or
model ID. A name edit preserves them. Explicit saved overrides survive refresh;
metadata refresh also preserves configured capacities and tool/image choices.
Old stored models without controls still deserialize. SQLite round trips and
foreground/background child snapshots retain both control metadata and Unknown
tool support. Provider inference probes use the configured token field too.

The token field must match the protocol: Responses uses `max_output_tokens`,
Claude uses `max_tokens`, and Chat Completions supports the configured
`max_tokens` or `max_completion_tokens`. The form keeps an incompatible saved
choice visible and explains how to select a valid field.

## Child output constraints

Both foreground and background child drivers attach the exact requested schema
when configured support enables native constraints:

| Backend | Request field |
|---|---|
| Owned llama.cpp runtime | `response_format: {type: "json_object", schema: ...}` |
| Chat Completions | `response_format: {type: "json_schema", json_schema: {schema: ..., strict: true}}` |
| Responses | `text.format: {type: "json_schema", schema: ..., strict: true}` |
| Claude Messages | `output_config.format: {type: "json_schema", schema: ...}` |

Unknown/unsupported remote structured-output support retains full answer
validation without claiming provider constraints were requested. Native requests
do not replace post-response validation: refusals, token limits, invalid JSON or
schema mismatches still fail. No schema is silently weakened or rewritten.
OpenAI strict-mode structural requirements are checked before the request:
an object root, closed objects and all declared properties required. Adjust an
incompatible schema or disable native constraints for post-validation mode.
Endpoints/runtimes can reject additional unsupported schema keywords.

## Acceptance status

Implemented as a draft. Offline tests cover protocol mapping, strict-schema
rejection, malformed payloads, provenance pins, discovery/override preservation,
SQLite and exact parent/child reconstruction. Actual foreground and background
drivers reject incompatible constraints before network access. A full loopback
fixture exercises native fields and successful/mismatched answers for local,
Chat Completions, Responses and Claude paths; the coordinator's sandbox denies
socket creation, so that fixture requires allowed host execution. It remains
unignored. Final independent SWE-2 Max review and fresh UI captures are pending.
No dependencies were added. Remote preflight counts remain labeled estimates;
the owned runtime still supplies its tokenizer count.

Sources: [OpenAI structured outputs](https://developers.openai.com/api/docs/guides/structured-outputs),
[OpenAI reasoning](https://developers.openai.com/api/docs/guides/reasoning),
[Claude structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs),
[Claude effort](https://platform.claude.com/docs/en/build-with-claude/effort),
and [llama.cpp server protocol](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md).
