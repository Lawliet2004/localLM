# Development tools completion matrix

Updated 2026-10-20. Core implementation increments and real acceptance are tracked
separately. **Overall acceptance remains incomplete.** Configured workflows,
registered tools and deterministic fixtures are not real dependency/platform
acceptance. Existing substantive uncommitted work was retained.

| Family | Implemented in this increment | Evidence on this Linux host | Remaining acceptance / limits |
|---|---|---|---|
| Debugger | Managed CodeLLDB reverse TCP connection, retained loopback listener, random-token handshake, separate diagnostic drains, framing/startup/lifetime bounds, autonomous protocol-failure cleanup; existing stdio, approvals and generation-bound references retained | Stdio/TCP lifecycle, authentication refusal, partial-handshake deadline, startup timeout, cancellation cleanup and reference tests | Real CodeLLDB and debugpy unavailable; Windows/macOS lifecycle unexecuted |
| Semantic edits | Case-only file renames coalesced into one native journal entry; requested spelling, rename-plus-edit, all-file preflight, parked original and conflict-preserving recovery | Both spelling directions, rename-plus-edit, distinct-file collision, stale hash, failed commit, prepared/parked/installed interruption and external-edit recovery | Native case-insensitive Windows/macOS acceptance unexecuted; parent-directory spelling changes remain unsupported |
| Verification | Supervisor-owned completion hook installed before spawn; observation starts before snapshots; immutable autonomous completion evidence; endpoint/membership checks; effective execution identity; queued event barrier and overflow/error handling | No-report-polling completion test distinguishes during-run modify/restore from later edits; historical observations remain unchanged; current freshness is independently checked | Linux queue ordering used for certification. Windows/macOS completion evidence remains conservatively incomplete pending a validated backend barrier; no verified-current claim there |
| Impact | Isolated pinned Git baseline via read-only Git plumbing; deleted-file and removed-symbol queries; separate baseline sessions, target output directory and offline Cargo environment; transitive call relationships; Cargo inheritance/target declarations; local packages; TS config inheritance/base URLs/aliases | Baseline preserves user tree/index/branch; deleted/removed symbols and indirect consumers; Cargo target inheritance; single/multiple TS inheritance and sibling-prefix fixtures | Real Rust/TypeScript servers unavailable. Dynamic dispatch, external consumers, unresolved package/config aliases and exhausted budgets remain explicit; complete graphs are never claimed |
| HTTP/OpenAPI | Productive recursive local bundles; invalid non-descending cycle refusal; nested schema ID/base handling; offline bounded validation work; explicit no-retry policy; wildcard status/media selection and truncation errors | Two real HTTP acceptance tests, including redirects, cross-origin refusal, vault credentials, cookie retention/limits, forms, multipart files, local/allowlisted remote recursive refs, required/invalid input, schema mismatch, truncation, persisted redaction and uncertain POST | Anchors/dynamic-reference dialect constructs and unsupported resource/serialization forms fail clearly; these are documented supported-format limits |
| Databases | Atomic intent/discovery journal; transactional PostgreSQL role marker / atomic MySQL user attribute; remote ownership rechecks; bounded administration and metadata; constraint/index/enforcement evidence; MySQL QUICK checks; bounded source preservation; logging disabled for credential-bearing driver statements | Real disposable PostgreSQL query/rehearsal, deferred constraint metadata, migration/connection/partial-restore failure, cancellation, restart, lost-acknowledgement state, marker refusal and repeated cleanup; original source checked | Real MySQL 8.0.21+ acceptance unavailable. Native Windows/macOS utility execution unexecuted. Legacy unfinished ledgers without remote markers require manual ownership investigation |
| Profiling | Duration-triggered graceful owned stop; bounded cancellation cleanup; retained incomplete cancellation/export-failure evidence; original trace handles survive bounding; binary/version/workload/cwd provenance; perf period weights and module-qualified symbols; explicit native export limitations | Real Node Inspector; deterministic graceful flush/cancellation, early exit, callchain/weight/export-reference tests; bounded trace-handle test | perf recording permission-blocked; WPR/WPA and xctrace require native hosts. WPA structured stack export and xctrace explicit weight units are required when those fields are supplied; unknown layouts fail rather than being silently flattened |
| Acceptance / CI | Actual configurable CodeLLDB, MySQL, native-profiler and semantic-baseline acceptance paths; exact executable checks; zero-test detection; passed/failed/blocked/skipped/not-run records; strict CI; always-upload logs/reports | Strict runner executed and correctly exited nonzero for blockers; available HTTP/Node and isolated PostgreSQL acceptance executed | Three-OS workflow exists locally but is absent remotely (GitHub workflow lookup returned 404); no CI run is claimed |

## Verification evidence for this increment

The former 742/650 checkpoint is historical, not evidence for these changes.
Current results:

- `cd src-tauri && cargo test --lib`: **760 passed, 0 failed, 17 ignored**.
  Ignored tests are excluded from passing counts.
- `cd src-tauri && cargo check --lib`: passed. The two existing unused provider
  capability helper warnings remain.
- `npm run typecheck`: passed.
- `npx vitest run`: **650 passed, 1 skipped**; 97 test files passed, one skipped.
- Rust formatting checks for the implementation files and `git diff --check`:
  passed. Python syntax checks for the runner and LSP fixture passed.
- Real HTTP: **2 passed**. Real Node Inspector: **1 passed**.
- Real PostgreSQL: **1 acceptance test passed**, exercising the scenarios in the
  database row above against a new, owned, loopback-only disposable container
  from an already cached image. Its container and temporary utility adapters
  were removed. No user database or user credentials were accessed.
- Native perf probe: **blocked** by `perf_event_paranoid=4`; no OS permission was
  changed. No real native perf capture is counted as passing.
- No browser localhost preview or alternate browser/proxy route was attempted.

Machine-readable consolidated evidence and logs for this session are under
`/tmp/locallm-development-acceptance-20261020/`. `report.json` combines the final
unit checks with the separately provisioned PostgreSQL fixture result. The
standalone runner's report also records that PostgreSQL was not configured in
its separate unprovisioned invocation; that does not erase the isolated run.
Strict acceptance remains nonzero because required dependencies are blocked.

## Explicit implementation limits

- Non-Linux watcher completion is autonomous and persisted, but cannot certify a
  complete observation interval. `verifiedCurrent` stays false there. A native
  queue-drain implementation/certification is still needed to remove this limit.
- Baseline preparation currently requires the selected workspace to be the Git
  root. Analysis is bounded and conservative, not a complete language/runtime
  dependency graph. Baseline/current edges and lexical relationships are distinct.
- Schema anchors/dynamic-reference dialects, non-pointer fragments, unsupported
  native exporter layouts and source volumes beyond the documented budgets are
  rejected explicitly. They are not treated as successful partial validation.
- Database source preservation is a bounded endpoint observation, not isolation
  from external writers or a complete scan of arbitrary databases. PostgreSQL
  metadata and MySQL engine QUICK checks have different integrity scopes.
- Cancellation retention is explicitly incomplete; application termination can
  interrupt asynchronous recorder cleanup/evidence persistence. Missing evidence
  is never interpreted as a completed or compatible capture.

## External acceptance blockers

1. **CodeLLDB:** no installed adapter executable is available/configured.
2. **debugpy:** the available Python has no installed debugpy module.
3. **Rust LSP:** `/home/papan/.cargo/bin/rust-analyzer` is a rustup shim; running
   it reports `Unknown binary 'rust-analyzer' in official toolchain`.
4. **TypeScript LSP:** no installed language-server executable/CLI is available.
5. **MySQL:** no isolated disposable MySQL fixture or installed dump/restore
   utilities are configured. No existing user server/database was probed or used.
6. **Linux perf:** actual recording is refused by the host's
   `perf_event_paranoid=4`; no elevation or permission changes were attempted.
7. **Windows:** no native host for WPR/WPA, case-insensitive transactions,
   debugger process jobs, watcher ordering or database utilities.
8. **macOS:** no native host for xctrace, case-insensitive transactions, watcher
   ordering or database utilities.
9. **CI:** the local workflow is not present in the remote repository; it is
   configured but unexecuted. No remote code or workflow was published.

## Repeatable acceptance

```sh
python3 scripts/development-acceptance.py --unit --strict --output /tmp/locallm-acceptance
python3 scripts/development-acceptance.py --only http --only node --strict --output /tmp/locallm-available
```

`--only` explicitly skips unselected families. Without it, every real family is
required. Strict mode fails for failed, blocked or missing/zero-test acceptance.
Without `--unit`, deterministic suites are recorded as optional `not-run`.
Non-strict execution is a diagnostic convenience, not complete acceptance. CI
uses strict mode and uploads logs/report JSON even when the job fails.

Use actual installed absolute executables, not Windows `.cmd`/`.bat` shims:

- `LOCALLM_DEBUGPY_PYTHON`: Python containing debugpy.
- `LOCALLM_CODELLDB`: installed CodeLLDB adapter.
- `LOCALLM_CC`: GCC/Clang-compatible C compiler for native fixtures.
- `LOCALLM_RUST_ANALYZER`: installed server, not an unavailable rustup component.
- `LOCALLM_TYPESCRIPT_LANGUAGE_SERVER`: actual executable, or Node with
  `LOCALLM_TYPESCRIPT_LANGUAGE_SERVER_SCRIPT` pointing to the installed CLI.
- `LOCALLM_NATIVE_PROFILER`: perf, WPR or xctrace on its native host.
- `LOCALLM_PROFILE_EXPORTER` / `LOCALLM_WPA_PROFILE`: installed exporter and
  CPU export profile where required.

Database tests require a fresh, isolated, disposable `locallm_acceptance`
database on loopback, never a user database. PostgreSQL uses the fixture's
`postgres` user; MySQL uses its fixture's `root` user. Configure:

- `LOCALLM_ACCEPTANCE_PG_PORT`, `LOCALLM_ACCEPTANCE_PG_PASSWORD`,
  `LOCALLM_ACCEPTANCE_PG_DUMP`, `LOCALLM_ACCEPTANCE_PG_RESTORE`.
- `LOCALLM_ACCEPTANCE_MYSQL_PORT`, `LOCALLM_ACCEPTANCE_MYSQL_PASSWORD`,
  `LOCALLM_ACCEPTANCE_MYSQL_DUMP`, `LOCALLM_ACCEPTANCE_MYSQL_RESTORE`.

Passwords enter only fixture environment/native isolated vaults and never CLI
arguments or reports. The fixture creates `source_guard`, uses generated scratch
databases/roles, and removes them on success. The isolated server/container must
also be disposed by its fixture owner on assertion failure. Runtime application
code installs no tool, starts no privileged container and changes no OS policy.

## Implementation and review links

- [Development tools and supported limits](DEVELOPMENT_TOOLS.md)
- [Language servers and reviewed edits](LANGUAGE_SERVER_TOOLS.md)
- [Transaction journal and recovery](WORKSPACE_TRANSACTIONS.md)
- [Managed debugger](../src-tauri/src/debug_tools.rs)
- [Semantic resource edits](../src-tauri/src/semantic_edits.rs)
- [Native transaction journal](../src-tauri/src/workspace_patch.rs)
- [Observation lifecycle](../src-tauri/src/verification_observer.rs)
- [Semantic impact](../src-tauri/src/semantic_impact.rs),
  [isolated baseline](../src-tauri/src/semantic_baseline.rs),
  [configuration relationships](../src-tauri/src/semantic_dependencies.rs)
- [OpenAPI bundles](../src-tauri/src/openapi_tools.rs),
  [bounded validation](../src-tauri/src/tool_validation.rs)
- [Remote rehearsal](../src-tauri/src/database_rehearsal.rs)
- [Native profiler](../src-tauri/src/profile_tools.rs)
- [Acceptance runner](../scripts/development-acceptance.py),
  [strict workflow](../.github/workflows/development-tools.yml)

An early formatting invocation expanded formatting beyond the intended Rust
file. Formatting-only changes in previously clean files were restored where
provable; substantive existing edits were never reset, stashed or cleaned.
Remaining formatting churn in existing dirty Rust files should be accounted for
when reviewing the working-tree diff.
