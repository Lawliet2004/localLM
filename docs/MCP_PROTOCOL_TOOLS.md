# MCP resources and prompts

Selected live MCP sources now expose five native protocol tools when the server
advertises the corresponding capability:

| Tool suffix (under `__locallm_mcp_`) | Wire operation |
|---|---|
| `resources_list` | `resources/list` |
| `resource_templates_list` | `resources/templates/list` |
| `resource_read` | `resources/read` |
| `prompts_list` | `prompts/list` |
| `prompt_get` | `prompts/get` |

They use the normal `ToolView`/`AgentTool` registry, strict schemas, hashed
connector aliases, deferred discovery, approval, audit and cancellation paths.
The selected connector source owns every operation; a URI or prompt name cannot
switch peers. Selecting only an ordinary server tool does not automatically
select protocol tools. A protocol tool can also be selected individually.
Servers with only resources/prompts now connect without an erroneous `tools/list`
request. A server tool using the reserved `__locallm_mcp_` prefix is rejected
explicitly rather than silently replacing or retargeting its identity.

The `mcp_resources_prompts` capability gates assembly and stale dispatch.
Protocol reads retain conservative MCP approval classification and ordered
execution. Inherited detached peers fail explicitly and ask the child to use the
parent; disconnects invalidate live discovery and dispatch. Fail-closed policy
now also includes the previously omitted Git and LSP capability IDs.

## Retrieval contracts

Catalogs are fully collected under independent limits: 512 entries, 64 pages,
2 MiB accumulated JSON, bounded cursors and duplicate/cycle rejection. Model
pages contain up to 32 complete entries and a catalog SHA-256 pin. A mismatched
expected pin fails visibly. Public arguments are bounded to 32 KiB; results
are bounded to 256 KiB with an explicit error instead of silent truncation.
The normal artifact pipeline can further bound model-visible result delivery.

A resource read accepts an exact URI listed by that server. Dynamic reads
require an exact listed `templateUri` plus a fully expanded `uri` matching its
fixed literal structure. The client does not expand, navigate or fetch the URI
and does not open it as a local path. The server interprets template expression
semantics and enforces its own data authorization. This membership check is not
a general RFC 6570 expansion engine. `resultSha256` hashes the complete serialized
MCP resource result, not decoded binary file bytes.

Prompt retrieval checks the current listed prompt, missing required arguments,
undeclared names and bounded string values before `prompts/get`. Retrieved
messages remain untrusted result data: they are never appended to the system
prompt, installed as instructions or executed. Resource text and base64 blobs
also remain data; binary resource content does not automatically become a model
image. Native operation signatures ignore membership/hash proofs and normalize
page defaults so proof changes cannot bypass replay protection.

## Acceptance and remaining A14 work

The resource/prompt portion is implemented and covered by six real stdio tests,
including parent catalog assembly and actual nested dispatch denial before RPC.
The fixture covers source/individual selection, resources-only servers, templates,
prompt contracts, catalog cycles/duplicates/limits, oversized content, detached
and disconnected peers, namespace collisions and capability failures.

**Interactive continuation is now implemented as a draft.** `mcp_input.rs`
provides a typed request registry with per-call `SessionInput` ownership and a
`CallUi` live-emitter authority; `resolve_mcp_input` is the only resolver and is
bound to the host-generated request id. `ApprovalDialog` renders a dedicated
`McpInputDialog` with escaped server labels/descriptions and distinct accept/
decline/cancel actions — a server tool merely named `ask_user` cannot hijack the
channel. Both legacy `elicitation/create` (server-initiated client request) and
SDK `inputRequired`/MRTR continuation are supported for tool calls, resource
reads and prompt retrieval, with bounded rounds, bounded state size (contents
preserved verbatim), original-argument carry-over, and cleanup on cancellation,
timeout, disconnect and completion. Call authority is threaded through parent,
PTC, harness and foreground-child routes; detached/background clones carry no UI
authority and input requests there fail closed. URL mode, sampling and roots
remain unimplemented and unadvertised.

**Transport bounds are closed at the client layer.** `mcp_http.rs` wraps the
pinned SDK's public `StreamableHttpClient` so plain JSON success bodies, HTTP
error bodies and malformed-response paths are length-bounded before
deserialization, while preserving OAuth challenge/refresh, headers, session
handling, redirect behavior, SSE bounds and cancellation. The SDK marks only
its own private `BoundedSseStreamError` as a fatal SSE failure; the wrapper's
equivalent error is retryable, and the SDK default reconnect policy is
unbounded — so `connectors.rs` bounds `retry_config` (8 attempts) on the
streamable-HTTP transport, turning a persistent GET-channel failure (oversized
event or dead endpoint) into a visible error instead of a silent infinite
reconnect. Stdio retains its 4 MiB frame guard. The byte-stream bound parser
is covered by offline tests, and `bounded_http_client_enforces_wire_limits_and_header_parity`
verifies the client over real loopback HTTP (bounded success/error bodies,
SSE event bound on POST and GET, session/auth/custom-header parity, JSON-RPC
error passthrough, delete-session 405 tolerance). A full remote `connect()`
HTTP handshake is bounded to real preset endpoints (no custom remote URLs).
Final independent SWE-2 Max and merged host HTTP acceptance remain pending.

The existing `ask_user` dialog's blank-submit fallback was removed while tracing
the interactive path. A blank response no longer chooses the first option or
"yes"; the user must click an option or enter an answer. A server tool merely
named `ask_user` cannot choose the host interaction channel: it retains ordinary
tool approval. Both behaviors have explicit frontend regressions.

Protocol references: [resources](https://modelcontextprotocol.io/specification/2025-11-25/server/resources),
[prompts](https://modelcontextprotocol.io/specification/2025-11-25/server/prompts),
[current elicitation](https://modelcontextprotocol.io/specification/2026-07-28/client/elicitation),
and [version compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning).
The implementation uses the installed `rmcp` 3.2.0 interfaces. No dependency
was added in this resource/prompt pass.
