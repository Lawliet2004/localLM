# Authorized tool discovery

LocalLM advertises up to 64 function schemas per model request. It removes aliases covered by a saved tool denial before freezing the catalog. A later tool allow or ask can restore an alias. A later path, command, or host exception keeps the corresponding tool available so dispatch can evaluate the actual arguments. Foreground and background children apply the same filter to their inherited tools.

Large catalogs add `tool_search` and `tool_invoke`. Coding turns also use this pair to defer specialist tools. When a saved rule disables either wrapper, permitted tools are offered directly up to the 64-schema limit. The turn shows a notice if more tools are selected. Searches recheck the current rules, and a changed result set invalidates its pagination hash. The advertised schemas stay fixed throughout the turn.

On local models, LocalLM checks whether the instructions, draft, tool definitions, and reply reserve fit before adding any chat history. If the catalog would overflow, it keeps essential tools such as web search directly available and defers the remaining schemas through discovery. The composer and generation use the same catalog selection.

`tool_search` accepts query (required string; empty lists all), offset0–4096, limit1–10 and optional expected_catalog_sha256. Search matches names, connector names and descriptions. Results include exact invocation names, descriptions and full input schemas. They are drawn from the current turn's selected sources/tools, re-filtered through capability policy and live-peer availability. A pinned page fails explicitly if the authorized catalog/query/schema changes. Tool descriptions are untrusted metadata. Oversized results use the normal saved-result artifact path; read the exact schema with artifact_read before invocation.

`tool_invoke` accepts a name and arguments object following the discovered input schema. The parent or child driver resolves it to the original borrowed tool before normal argument preparation, capability checks, read/write classification, approvals, audit, execution, cancellation and result bounding. It does not grant a blanket approval or clone a live MCP peer into a detached marker. Invalid target arguments fail before dispatch. Unknown/unselected tools and recursive wrappers fail visibly; finish, update_context and ptc_run must be called directly. Direct calls to the harness invoker outside a resolver fail closed.

Example after selecting a connector whose catalog contains a deferred tool:

```json
{"name":"tool_search","arguments":{"query":"reference","limit":5}}
```

Use a returned exact `tools[].name` and its `input_schema`:

```json
{"name":"tool_invoke","arguments":{"name":"<exact returned name>","arguments":{"<required field>":"<value>"}}}
```

The wrapper's arguments object deliberately permits tool-specific fields; the resolved tool's own strict schema validates them before approval or execution. There is no executable, source-selection or permission override in the wrapper.

Foreground children search their filtered borrowed catalog; detached children search only the filtered tools reconstructed from their authorized snapshot. Detached live-only tools remain unavailable and disclosed through existing child limits. Their original approval pins still apply; discovery does not provide an approval interface to a background run.

Limits remain explicit:4096 authorized/selected tools,8MiB aggregate serialized schemas,64 connector source IDs,512 tools and2MiB metadata per connected MCP server,64 protocol discovery pages,32KiB invocation arguments,64 advertised schemas, bounded result pages/artifacts. Duplicate aliases, reserved wrapper collisions and excessive catalogs fail rather than silently dropping tools. The existing MCP pagination helper and its cursor/duplicate/page/byte bounds remain separate and unchanged.

Validation includes deterministic advertisement/core controls, pinned paging, source/capability/argument rejection, selected-tool persistence limits and a real two-page70-tool stdio MCP fixture with disconnect handling. A scripted real-parent model test checks discovery of an actually deferred selected tool, rejection of malformed arguments, exactly one live MCP invocation, durable audit and identical64-schema payloads across four rounds. That full model-round test requires a loopback-capable native test environment; the coordinator's restricted sandbox currently denies socket binding. Do not infer it passed from the stdio/catalog tests. Acceptance and final merged checks are tracked in INFERENCE_TOOL_IMPLEMENTATION_PLAN.md.
