# Daytona execution implementation contract

Current status: Daytona is selectable in chat after saving a key, with code approval in Ask and Auto-approve reads modes. The runtime attempts ownership-checked cleanup after execution/cancellation and once at startup for saved pending operations. Native denial tests passed in both prompting modes with no cloud resources created. Live account execution, credential rotation with pending cleanup and native restart recovery acceptance remain unverified. Implementation sections below record the stages that preceded this wiring.

Startup recovery holds the cloud operation lock, skips other credential scopes without network requests, and retains per-operation failures while continuing to other records. Missing credentials are recorded as actionable cleanup errors. Recovery never reruns code or creates resources. A mixed-scope fixture verifies failed cleanup survives, unrelated credentials are skipped and confirmed absence clears the matching record.

## Sources inspected on 2026-09-08

- [Official API reference](https://www.daytona.io/docs/en/tools/api/).
- [Platform OpenAPI](https://www.daytona.io/docs/openapi.json), SHA-256 `fb9d877c6bbedb818b257a5a9fa6837ac97f5ce3b24c00a4ba7effb6f195178c`.
- [Toolbox OpenAPI](https://www.daytona.io/docs/toolbox-openapi.json), SHA-256 `3dbcb22f53b0205deedc9310dc2e87aa346cf28db9372508c28919b70a13b3f6`.

The platform API creates sandboxes with POST `/sandbox`, inspects/deletes them at `/sandbox/{sandboxIdOrName}`, and provides a toolbox endpoint through GET `/sandbox/{sandboxId}/toolbox-proxy-url`. Bearer authentication applies to platform requests. Creation supports private previews, resource limits, auto-stop, auto-delete and a wall-clock TTL.

The toolbox POST `/process/code-run` request requires both `code` and `language`. Documented languages are Python, JavaScript and TypeScript. Optional fields include argv, envs and timeout; results include exitCode, result and artifacts. Do not infer separate stdout/stderr streams from this combined result.

## Transport implementation

`src-tauri/src/daytona.rs` implements create/inspect/delete/code-run requests. It limits responses to 1 MiB, disables redirects, bounds request durations and returns status-only errors instead of reflecting remote error bodies. Code accepts Python/JavaScript/TypeScript, 32 KiB maximum and 1–90 seconds. Creation requires a LocalLM operation name, requests private previews, and sets explicit resource/lifetime limits. No HTTP mutation is automatically retried by the application.

The client accepts only the hosted `https://proxy.app.daytona.io/toolbox` origin/path before attaching credentials. Regional or self-hosted endpoints currently fail explicitly. Proxy fallback lookup, durable ownership, readiness polling, cleanup recovery, credentials UI and tool integration remain outstanding. The transport alone must not be exposed as an executable chat tool before those lifecycle pieces exist.

SDK details were checked at [daytona/clients revision 246056df](https://github.com/daytona/clients/tree/246056df8886a020396cb46ad17c1d8b93653648/sdk-typescript/src): Process.ts uses seconds for execution timeout; Daytona.ts configures Bearer authorization; Sandbox.ts appends the sandbox ID to the proxy base URL. Real account compatibility still needs verification.

Local TCP fixtures verify authentication, creation JSON, explicit ownership/lifetime settings, missing sandboxes, redirect rejection, oversized responses and exclusion of credentials/remote error text from returned errors. URL/key validation tests reject unsafe origins, paths and header injection.

## LocalLM design decisions to implement

These are application decisions, not claims about completed functionality:

1. Rust owns authentication and HTTP requests. Store the API key in the existing encrypted vault; never return it in bootstrap data, logs or model context. The UI shows only credential presence.
2. Add a separately selected Daytona tool and provider settings. The approval view shows code, language and cloud execution scope. Ask and Auto-approve require approval; Full access permits the selected tool. Enabling local execution must not enable Daytona implicitly.
3. Create one private sandbox per run with explicit lifetime/resource limits. Persist a locally generated operation identifier before creation and record the returned sandbox ID immediately. Resolve ambiguous creation by identity; do not blindly retry resource creation.
4. Poll readiness within a bounded deadline, execute once, bound response bytes, then delete. Cancellation must trigger cleanup outside the cancelled generation future. Failed cleanup stays visible and durable for recovery on the next launch. A dropped HTTP request is not proof that cloud execution stopped.
5. Do not upload workspace files implicitly. File transfer and artifact retrieval need explicit paths and the same authorization/audit model. Returned artifact content is untrusted.
6. Test authentication, response limits, invalid proxy origins, timeouts, cancellation during every lifecycle phase, ambiguous creation and failed cleanup using local HTTP fixtures. Real-account verification remains separate and requires credentials entered in the application.

Implement the native client and durable cleanup record first, then provider settings, tool selection, approval presentation and live acceptance. This work does not narrow the existing Daytona requirement to configuration-only support.

## Durable operation journal

The app now opens a separate daytona.sqlite journal in its data directory. Records store an operation name, credential scope identifier, optional sandbox ID, creation time and sanitized cleanup error. A name is committed before creation; its first sandbox association cannot later be reassigned. Acknowledging verified remote absence requires the matching credential scope. Conversation deletion does not cascade into this separate journal.

A read-only native command exposes pending records to the Execution page. Reopen tests verify unknown creation outcomes and failed cleanup remain, conflicting sandbox association fails, and mismatched scopes cannot clear ownership. UI tests verify pending errors and journal-read failures are visible. The native empty-journal/page check passed. The journal is infrastructure for the upcoming lifecycle manager: automated recovery and actual cloud execution are not yet wired.


## Cleanup recovery engine

The recovery engine validates the credential scope locally, inspects the saved sandbox ID (or operation name for an ambiguous creation), and requires the returned name/ownership label/known ID to match. It persists a discovered ID before DELETE. It then polls for absence, with a 60-second overall deadline. Failed or interrupted deletion retains the record; acceptance of DELETE does not acknowledge cleanup. An unknown creation that is not yet visible remains unresolved rather than being cleared prematurely.

Tests cover successful deletion, ownership mismatch, API failure, unresolved creation and scope mismatch. A cancellation test interrupts deletion after association and reopens SQLite to verify identity survives. This engine is not yet called automatically or from a recovery button; credential integration and lifecycle wiring remain next.


## Credentials and manual cleanup UI

Execution now saves/forgets a Daytona API key through the shared encrypted vault and exposes only a presence flag. Saving validates format locally but does not verify the account or allocate resources. Pending records prevent replacing the associated key with a different key or forgetting it. Credential scope currently hashes the key; rotation of an expired key while cleanup remains is not yet supported and needs verified account-identity handling before release.

Retry cleanup invokes the ownership-checked recovery engine for the selected record. It serializes cloud operations and refuses to run during a model operation. The UI refreshes pending records after success or error. Automatic startup recovery and the executable cloud tool remain incomplete.

The native daytona-settings smoke test refuses to replace an existing key. Using a generated fixture, it verifies encrypted-file contents do not contain the key, input clearing, persisted presence across reload and forgetting, without cloud calls. The test restores the no-key state.


## Execution lifecycle coordinator

The native coordinator now validates code arguments, serializes a cloud run, journals ownership before creation, records returned identity, waits for started state, submits code once and attempts cleanup. It returns execution output/errors and cleanup errors separately, with an overall isError flag. Nonzero exit codes are retained as failed execution results rather than transport failures.

The caller owns a cancellation sender; dropping it signals a separately spawned worker. Creation is allowed to return within its transport timeout so identity can be captured. After creation, cancellation skips or interrupts code submission and proceeds to cleanup. Cleanup is not tied to the dropped chat future. Interrupted/failed cleanup leaves its durable record. A process crash can still interrupt the worker, which is why startup recovery remains necessary.

A fake remote verifies ownership exists before creation, one code submission, success/nonzero results, and cleanup after caller cancellation. Real Daytona execution and chat-tool wiring are still unverified/incomplete; the coordinator is not yet exposed to the model.

