# Development tools

The development-tool catalog extends the existing harness with debugging,
semantic edits, verification evidence, impact analysis, HTTP/OpenAPI workflows,
databases and profiling. All calls retain source selection, capability,
approval, audit, versioned tool-envelope and artifact boundaries. There are now
32 additional aliases relative to the original catalog. The everyday catalog
has 64 schemas; authorized deferred tools remain available through discovery.

Implementation and acceptance status are tracked in
[DEVELOPMENT_COMPLETION.md](DEVELOPMENT_COMPLETION.md). That matrix includes
remaining implementation gaps; a registered tool is not proof of acceptance.

## Permissions and settings

Use **Settings → Development tools** for debugger timeouts, profiler paths,
remote OpenAPI reference origins, HTTP credentials and database connections.
Native vault secrets are never returned by the settings API or tool discovery.
HTTP credentials are bound to an exact HTTP(S) origin. Tools see configured
credential/profile IDs and names, not passwords. An unknown ID cannot address an
arbitrary vault item. Supplying secrets directly as ordinary tool arguments
still exposes them to the existing argument audit store.

All families require a workspace. Debugging, verification execution, benchmark
execution, profile capture and remote rehearsal utilities additionally require
local execution. The existing shell approval policy applies and is rechecked
at dispatch. LSP commands and remote cleanup require explicit approval even in
Full access. Required LSP annotation IDs also trigger approval on application.
Capability kills remain effective at dispatch. Stateful development tools need
the foreground dispatcher; detached children cannot execute LSP commands.

Installed executables are user configuration; no runtime tool installs a
package, changes OS profiling permissions, or starts a privileged container.

## Debugging

`debugger_start`, `debugger_control`, `debugger_inspect`, `debugger_events` and
`debugger_stop` support installed **stdio** DAP adapters and managed **CodeLLDB TCP**. For debugpy:

```json
{"adapter":"/absolute/python","adapter_args":["-m","debugpy.adapter"],"configuration":{"program":"main.py","console":"internalConsole","stopOnEntry":true}}
```

Start returns `sessionId` during the configuration phase. Set source or
exception breakpoints, then send `configurationDone` when supported. DAP source
paths are workspace relative; line and column positions are one based.
Inspection supports threads, stack traces, scopes, variables and exceptions.
Frame and variable references are opaque, issued by LocalLM and invalidated
on resume and subsequent stops. Old or fabricated IDs are rejected.

Request, idle and total lifetime limits are independent of the shell execution
timeout. Defaults are 30 seconds, 15 minutes and two hours respectively.
Activity resets the idle deadline. Malformed frames, cancellation and timeout
retire the leased adapter tree. Events are bounded and disclose dropped
records. Reverse process-launch requests are refused. Disconnect does not ask
the adapter to terminate an independently attached debuggee.

For CodeLLDB, use `transport:"codelldb"` with an absolute adapter executable
and its ordinary adapter arguments. LocalLM binds and retains `127.0.0.1:0`,
launches the owned adapter with `--connect <port> --auth-token <random token>`,
and checks its initial `Auth-Token` line before accepting DAP. Transport switches
cannot be overridden through adapter arguments. This follows CodeLLDB's
[reverse-connect implementation](https://github.com/vadimcn/codelldb/blob/master/src/codelldb/src/lib.rs).
No free-port probe/rebind occurs. Diagnostic stdout/stderr is drained separately;
only the authenticated socket carries frames. Startup/authentication, partial
frames and requests have bounded deadlines; lifetime includes startup. Protocol
failure, EOF, cancellation, idle expiry and disconnect retire the owned process
resources. The same capability, approval and foreground-dispatch rules apply.
Real CodeLLDB/debugpy acceptance requires the explicitly configured installed
adapters; deterministic transport tests do not establish real compatibility.

## Semantic navigation and edits

`lsp_definition`, `lsp_hover`, `lsp_implementations`, `lsp_code_actions`,
`lsp_prepare_code_action`, `lsp_apply_edit` and `lsp_execute_command` extend the
existing diagnostics, symbols, references and rename tools.

Positioned requests use a configured server, workspace path, zero-based line,
UTF-16 character and current source hash. Code-action preparation pins both the
source and listed action result. Text edits and ordered create/delete/rename
resource operations are staged as owned proposals. No files change until
`lsp_apply_edit` or `lsp_rename` passes ownership, settings, versions, hash and
native transaction checks. Required change annotations must be confirmed by ID.
Case-only file renames, including subsequent text edits, use one native journal
entry. The original is parked under an owned transaction name; the requested
spelling is installed without clobbering a distinct destination. All-file
preflight still precedes the first mutation. Recovery covers either side of
that transition and preserves divergent external edits. Parent-directory
spelling changes are not file-only renames and remain unsupported.
Directory resource operations and unsupported encodings are also rejected.

Configure allowed command names per language server in native settings.
`lsp_execute_command` requires a current source hash, user allowlisting,
server advertisement and explicit approval. Server `workspace/applyEdit`
requests received during the command become proposals; the client reports
`applied:false` until separate application. Commands can also mutate files
independently: results disclose uncertainty and must never be blindly replayed.
Unsolicited server edit requests remain refused.

See [LANGUAGE_SERVER_TOOLS.md](LANGUAGE_SERVER_TOOLS.md) and
[WORKSPACE_TRANSACTIONS.md](WORKSPACE_TRANSACTIONS.md) for bounds and recovery.

## Verification and change impact

`verification_run` accepts check arguments and `mode`:

- `explicit` (default): requires 1–128 `source_paths` with the existing 8 MiB cap.
- `workspace`: inventories relevant source and build inputs, up to 4096 files
  and 64 MiB; exclusions and incomplete reads are reported.
- `affected`: selects current semantic and conservative package dependencies
  plus build manifests. An incomplete graph cannot yield complete coverage.

Evidence binds source hashes, command, execution settings, app version and
configured runtime binary fingerprints. A native file watcher observes writes,
including modify-then-restore activity. Observation begins before snapshots and
launch. A completion hook is installed in the owned process supervisor before
spawn, captures endpoint hashes/membership when the child finishes, and persists
an immutable observation independently of report polling. Later reports cannot
extend or rewrite the historical interval. Configuration fingerprints include
the effective check environment defaults and working directory.

Linux drains queued watcher events through an owned queue marker before sealing.
Overflow/rescan notices, watcher errors, missing completion evidence, uncertain
termination, endpoint changes and application restart fail closed. Events after
child exit but before the marker is consumed are conservatively included; this
can cause false positives. Windows/macOS observations currently remain incomplete
because the queue-marker ordering guarantee has not been established there.
Native watchers do not provide atomic isolation from external writers or cover
unselected dependencies. Reports independently recheck current hashes, and
workspace reports recheck membership, including new and deleted inputs.
`verification_report` never reruns checks and only reports `verifiedCurrent`
when recognized tests pass and all freshness, coverage and observation checks
succeed. Each report has an immutable ID. `verification_compare` compares
recorded reports without asserting causal equivalence or universal coverage.

`change_impact` defaults to semantic analysis for configured Rust and TypeScript
servers: symbols, references, implementations, callers and callees. Edges carry
server fingerprints and source hashes. Cargo path/workspace declarations,
workspace package dependencies and TypeScript project references add
conservative propagation. TypeScript config comments/trailing commas are
accepted. Explicit `mode:"lexical"` retains bounded heuristic search; automatic
mode may include that separately labeled evidence.

Deleted files and symbols removed from surviving files are analyzed in an
isolated, pinned `HEAD` tree. Git `ls-tree`/`cat-file` populate an owned temporary
directory; no checkout, worktree registration, index, branch or user file is
changed. Current and baseline LSP managers/sessions are separate. Baseline Cargo
outputs are directed to the owned directory, Cargo networking is disabled there,
and TypeScript automatic typing acquisition is disabled. Project settings rooted
in the current workspace are rebased. Edges identify current/baseline evidence,
commit, source hashes and actual server fingerprints; both endpoints are checked
for freshness. Preparation currently requires the workspace to be the Git root.

Budgets: 30-second baseline preparation, 120-second total analysis, 4096 source
files/64 MiB, 64 server requests, three transitive call-hierarchy levels, a
128-item expansion queue and 4096 returned edges. Cargo workspace dependency
inheritance and target-specific dependencies are included. Local package names,
`file:`/`link:` dependencies, TypeScript references, single/multiple config
inheritance, base URLs and single-wildcard aliases add conservative relationships.
Static import resolution is separately labeled lexical/configuration evidence.

No complete dependency graph is claimed. Missing servers/configs, unresolved
aliases/packages, dynamic dispatch, external consumers and exhausted budgets
remain explicit. Incomplete analysis broadens affected source selection to the
relevant workspace inventory and cannot yield complete verification coverage.
Candidate tests are evidence, not automatic execution.

## HTTP and OpenAPI

`http_session_create` creates an origin-bound pooled session with default
headers, optional `credential_id` and an in-memory cookie jar. Cookie ingestion
is capped at 256 entries and 4096 bytes per cookie. Session close discards the
client and credentials. Cross-origin session requests are rejected.

`http_request` supports JSON, raw text, URL-encoded forms and workspace-file
multipart bodies. Redirects are disabled by default; `follow_redirects` allows
at most five same-origin redirects. Set session redirect policy at creation.
Requests have a 30-second timeout and 1 MiB response cap; multipart fields and
files share an 8 MiB input budget. Truncation is an error even without a schema.
Client retries are explicitly disabled. Offline mode blocks network access. HTTP errors and schema
mismatches are explicit; uncertain writes are never retried automatically.

`openapi_inspect` and `openapi_request` read OpenAPI 3.0/3.1 JSON or YAML.
References resolve within the workspace, or from exact native-allowlisted
remote origins. Productive recursive schemas become bounded local `$defs`
bundles; they are not recursively inlined. Reference-only/non-descending cycles
and cyclic OpenAPI Reference Objects fail explicitly. Nested OpenAPI 3.1 schema
`$id` bases and document-relative references are preserved before rewriting.
Validation never retrieves resources after authorized resolution.

Limits: 1 MiB/document, 32 document/resource identities, 8 MiB cached content,
512 references, 128 schema definitions, 32 structural levels and 45 seconds for
resolution. Compiled schemas have a 64 KiB/8000-node limit; instance depth/nodes
and a conservative 100,000-step evaluation-work budget bound recursive
validation. Unsupported anchors/dynamic references and resource-ID conflicts
fail explicitly. OpenAPI 3.0 nullable/exclusive bounds are converted, its `$ref`
siblings are ignored as specified, and boolean schemas require 3.1. OpenAPI 3.1
schema `$ref` siblings retain their constraints. Request construction validates server variables,
required inputs and body schemas; path/query/header/cookie styles are encoded.
Response schemas are compiled **before** sending, then selected by response
status and media type. Unsupported schema or serialization forms fail clearly.

Response artifacts redact credential keys and known supplied credential values.
Redaction of arbitrary unknown secrets remains best effort. `http_compare`
provides bounded structural JSON-pointer changes, including type changes,
against owned response IDs.

## Databases

`database_schema`, `database_query`, `database_explain` and `database_rehearse`
accept a workspace SQLite `path` or native `profile_id` for PostgreSQL/MySQL.
Queries support bounded scalar parameters and 1–200 rows. External database
profiles should use accounts restricted to read access; read-only transactions
are defense in depth, not isolation from privileged extensions.

SQLite snapshots use the online backup API, including committed WAL contents,
without changing source journal mode. No-follow file/sidecar identity checks,
32 MiB snapshot bounds, authorizers, progress limits and output caps apply.
Migration rehearsal uses a private snapshot and reports schema/integrity/foreign
key results. No apply-to-original operation exists.

PostgreSQL/MySQL use native encrypted profile credentials and server-enforced
read-only transactions. PostgreSQL sets a statement timeout; MySQL sets a query
execution limit. Errors/timeouts drop or roll back the transaction.

Remote rehearsal requires a **separate scratch-administration profile** plus
explicitly configured installed dump/restore utilities. It clones into a random
database with a random restricted role, executes migration SQL through the
driver, compares schema metadata, and cleans up only recorded owned resources.
MySQL DDL's implicit commits are disclosed. Remote integrity evidence describes
server constraint/metadata checks, not physical corruption scanning.

Creation intent and the cleanup-discovery index are committed together before
remote creation. PostgreSQL role creation and its ownership comment share a
transaction; MySQL 8.0.21+ creates its ownership attribute with the temporary
user. Cleanup checks the remote marker, generated names, recorded absence before
creation and (PostgreSQL) database ownership, in addition to the original admin
profile fingerprint. Lost acknowledgements remain discoverable and do not
require replaying creation. Marker mismatches are preserved, never adopted.
Legacy unfinished ledgers without remote markers remain discoverable but require
manual ownership investigation; automatic destructive cleanup is refused.
`database_cleanup` with no ID discovers interrupted owned rehearsals; repeated
cleanup of a completed record is safe. Cancellation can leave cleanup work and is never reported as an
undone remote write. Dump bytes are hashed for provenance; source databases are
never the migration target. Source preservation is checked through read-only
before/after schema and row hashes, bounded to 16 tables and 200 returned
columns/rows. Incomplete or unavailable observations return `originalModified:
null`, rather than claiming the source was unchanged. These are endpoint
observations, not isolation from external writers.

Scratch metadata is capped at 4096 columns/constraints/index records. PostgreSQL
reports validation/deferral, index validity and non-default trigger enforcement.
MySQL reports keys, check clauses, referential rules, storage engines and session
constraint switches; engine `CHECK TABLE ... QUICK` evidence covers at most 16
scratch tables. These checks have their stated scope and are not exhaustive
physical corruption scanning. Deferred transactions are committed inside scratch
before success is reported. Administration, clone utilities, migration execution
and cleanup/ownership inspection are bounded. Driver statement logging is disabled
to keep generated role passwords out of logs. Privileged extension effects are
not OS-isolated.

## Profiling and performance

`benchmark_run` retains supervised warmup/measured samples, failures, source
snapshots and environment metadata. `benchmark_compare` includes a deterministic
2000-resample percentile bootstrap interval for median percentage change, with
at least two successful measured samples per group. It never establishes
causality or hardware isolation.

`profile_capture` supports an owned Node Inspector CPU session and configured
native perf, WPR/WPA or xctrace capture. It retains original exports and bounded
raw trace artifacts, plus normalized profiles, source/settings/runtime hashes
and measurement units. Captures also fingerprint the workload executable,
profiler/exporter binaries and their bounded version probes, working directory
and host identity. Missing provenance is explicit and comparisons reject
incomplete provenance or retention; differing workload binaries are disclosed.
Native trace retention is capped at 5 MiB/128 files.
Requested native duration is a normal stopping condition. Owned Unix recorders
receive SIGINT and up to ten seconds to flush; exporters start only after recorder
completion. WPR uses an owned instance name for stop/cleanup, and xctrace retains
its time-limit behavior. Cancellation performs bounded owned cleanup and persists
an explicitly incomplete capture/retention record; application termination can
prevent that asynchronous evidence from being saved. Export failure retains
available raw traces with an incomplete result. Oversized retention is explicit.

Linux perf records the owned workload with `cpu-clock:u` and preserves callchains and exported period weights in
nanoseconds; unweighted imports remain sample counts. WPA requires Function and
Weight (ms) columns. An optional `Stack`/`Stack (JSON)` column accepts bounded
root-to-leaf JSON symbol arrays; unsupported stack encodings fail instead of
silently discarding callchains. Leaf-only exports are explicitly labeled.
xctrace resolves backtrace/frame/weight references and retains explicit weight
units. Missing references, mixed units or weights without explicit units fail.
Unweighted exports remain sample counts, not CPU time.

`profile_inspect` supports Node CPU JSON, Speedscope sampled/evented profiles
and Chrome complete events. Evented stack consistency and timestamps are
validated. Chrome nested/concurrent duration totals are not elapsed wall time.
`profile_compare` compares compatible owned captures by symbol/file identity,
rejects incompatible workload/settings/runtime units, and reports stale sources
and symbolization limitations. Native OS capture acceptance remains pending.
