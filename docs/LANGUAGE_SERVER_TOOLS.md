# Language server tools

Additional navigation and semantic-edit tools are documented in
[Development tools](DEVELOPMENT_TOOLS.md#2-semantic-navigation-and-refactoring):
`lsp_definition`, `lsp_hover`, `lsp_implementations`, `lsp_code_actions`,
`lsp_prepare_code_action`, `lsp_apply_edit`, and `lsp_execute_command`. Code actions use the same
proposal ownership, file/configuration hashes and recoverable transactions as
rename. Selected ranges, diagnostic context, reviewed resource operations and
explicitly approved allowlisted commands are supported.

Settings → Execution → Language servers appears alongside Bash and Python. Configure up to eight already installed stdio servers: a unique ID, language ID, absolute executable, argument array, environment, initialization options, server settings and a 1–120 second timeout. Save before Test; Test initializes the saved executable in the current conversation workspace. No server packages are installed. On Windows, select the actual executable (for example node.exe plus a server script argument), rather than relying on shell shim expansion.

Enable language server tools and select the workspace source for a compatible conversation profile. The `lsp_tools` capability depends on `workspace_files` and `local_execution`; disabling any dependency denies dispatch. Disabled or unavailable executables are omitted with a configuration notice. The model sees enabled server IDs, never a command override. Always request approval is enforced even in Full access, including after settings change between offering and dispatch. Rename application remains a write under the existing conversation approval policy.

## Registered tools

`src-tauri/src/lsp_tools.rs::registry()` follows the harness `(name, description, input_schema)` registration interface. Each input is an object with `required` and `additionalProperties: false`; hashes and positions have strict bounds. `harness.rs` registers these definitions and dispatches them to `lsp_tools::execute`; the existing tool-envelope, capability, approval, audit and artifact paths remain in use.

| Tool | Input | Result |
|---|---|---|
| `lsp_diagnostics` | server, path; optional source hash and page parameters | Current-document diagnostics; `clean` is true only for a complete verified empty report. Unversioned push evidence is `incomplete`, with `clean: null`. Missing/stale/malformed/unsupported evidence fails explicitly. |
| `lsp_symbols` | server, path; optional source hash and page parameters | Validated hierarchical document symbols with source/result hashes. |
| `lsp_references` | server, path, zero-based line and UTF-16 character; optional include_declaration and page parameters | Validated workspace locations, with explicit omitted external-location count. |
| `lsp_prepare_rename` | server, path, line, character, new_name, expected_sha256 | Conversation/workspace-owned proposal ID/hash, file hashes and diffs. No project files changed. |
| `lsp_rename` | server, proposal_id, proposal_sha256 | Native all-file preflight and recoverable transaction result. Changed files/settings or foreign proposals are rejected. The server is never rerun during application. |

Read pagination uses offset (0–10000), limit (1–200, default 50) and optional expected_result_sha256. The result pin binds server configuration, workspace, source, method, position and declaration choice as well as result records. Repeat with the source and result hashes before advancing. Invalid records and excessive nesting fail rather than silently becoming an empty result.

Example call against the local test fixture (the required source hash is obtained from the current file):

```json
{"name":"lsp_references","arguments":{"server":"fixture","path":"main.rs","line":0,"character":3,"limit":1}}
```

The fixture returns one workspace location and one external location. The model-visible canonical result has status `success`, data containing `server: "fixture"`, `path: "main.rs"`, `total: 1`, `omittedExternalLocations: 1`, the exact source/result SHA-256 values and one item with `path: "main.rs"`. The external location is omitted. This is a fixture example, not a claim that a server is installed in the user's workspace.

## Rename review and bounds

Preparation pins every affected file and the source document. Full diffs (including CRLF and missing-final-newline evidence) and proposed content remain in the stored proposal. Display diffs above 16 KiB explicitly set `diffTruncated` and `reviewIncomplete`; model-result bounding also preserves the exact proposal ID/hash at the supported 512-byte floor. Read the proposal artifact before applying a clipped preview. Application verifies artifact ownership/integrity, canonical workspace, server configuration and all file hashes through the existing native transaction backend. An external process can still race ordinary filesystem operations; this is not an operating-system filesystem isolation guarantee.

Create/delete/rename file operations are staged with text edits. Case-only file renames (including subsequent text edits) use one recoverable native journal entry and preserve the destination spelling on sensitive/insensitive filesystems. Distinct case-colliding destinations are refused. Directory operations and parent-directory spelling changes remain unsupported. Commands require native allowlisting, server advertisement and explicit approval; client-mediated edits received during them are staged. Unsolicited server edits are refused. Required annotation IDs must be confirmed through approval; unknown IDs are rejected. Maximums: 16 files, 1024 edits per file, 1 MiB per source/result file, 8 MiB aggregate affected content and 16 MiB serialized proposal. Oversized operations fail as a whole. Overlapping text edits, unsupported encodings, malformed ranges, protected paths, links, pending recovery journals and stale document versions fail explicitly.

## Lifecycle and validation

The bounded JSON-RPC client negotiates UTF-16 positions, initializes the configured server, opens current source snapshots and serializes requests per session. Document queries require open/close synchronization support. Four owner/workspace/configuration-isolated sessions are allowed, with two-minute idle eviction. Stdio frames are bounded, stderr is drained with bounded retention, notifications/server requests have limits, and unsupported client methods receive explicit protocol errors. Configuration changes retire old sessions; timeout, Stop or a protocol error drops/kills the lease so an outstanding reply cannot contaminate a subsequent call. Managed process groups/Windows jobs are reused; normal shutdown sends shutdown/exit before bounded forced cleanup.

The Python standard-library fixture exercises real stdio, configuration requests, edit refusal, symbols/references, pull and push diagnostics, version checks, multi-file rename/preflight, stale settings/approval/capability/ownership, malformed records, result pins, oversized proposals, cancellation and Linux descendant cleanup. No live server, network service, credentials or new application dependency is required. Windows lifecycle code has not been run on Windows in this environment.

Current evidence and outstanding review are tracked in INFERENCE_TOOL_IMPLEMENTATION_PLAN.md. Fresh browser visual verification is pending because the local-preview origin approval was declined; existing screenshots are reference evidence only. The expanded Creator catalog also needs the planned A11 discovery integration before the merged implementation is accepted.
