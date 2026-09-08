# Daytona execution implementation contract

Status: researched, not implemented. Local execution remains the only available provider. No cloud resources were created during this research.

## Sources inspected on 2026-09-08

- [Official API reference](https://www.daytona.io/docs/en/tools/api/).
- [Platform OpenAPI](https://www.daytona.io/docs/openapi.json), SHA-256 `fb9d877c6bbedb818b257a5a9fa6837ac97f5ce3b24c00a4ba7effb6f195178c`.
- [Toolbox OpenAPI](https://www.daytona.io/docs/toolbox-openapi.json), SHA-256 `3dbcb22f53b0205deedc9310dc2e87aa346cf28db9372508c28919b70a13b3f6`.

The platform API creates sandboxes with POST `/sandbox`, inspects/deletes them at `/sandbox/{sandboxIdOrName}`, and provides a toolbox endpoint through GET `/sandbox/{sandboxId}/toolbox-proxy-url`. Bearer authentication applies to platform requests. Creation supports private previews, resource limits, auto-stop, auto-delete and a wall-clock TTL. The proxy authentication details and timeout units require verification against the upstream SDK before implementation.

The toolbox POST `/process/code-run` request requires both `code` and `language`. Documented languages are Python, JavaScript and TypeScript. Optional fields include argv, envs and timeout; results include exitCode, result and artifacts. Do not infer separate stdout/stderr streams from this combined result.

## LocalLM design decisions to implement

These are application decisions, not claims about completed functionality:

1. Rust owns authentication and HTTP requests. Store the API key in the existing encrypted vault; never return it in bootstrap data, logs or model context. The UI shows only credential presence.
2. Add a separately selected Daytona tool and provider settings. The approval view shows code, language and cloud execution scope. Ask and Auto-approve require approval; Full access permits the selected tool. Enabling local execution must not enable Daytona implicitly.
3. Create one private sandbox per run with explicit lifetime/resource limits. Persist a locally generated operation identifier before creation and record the returned sandbox ID immediately. Resolve ambiguous creation by identity; do not blindly retry resource creation.
4. Poll readiness within a bounded deadline, execute once, bound response bytes, then delete. Cancellation must trigger cleanup outside the cancelled generation future. Failed cleanup stays visible and durable for recovery on the next launch. A dropped HTTP request is not proof that cloud execution stopped.
5. Do not upload workspace files implicitly. File transfer and artifact retrieval need explicit paths and the same authorization/audit model. Returned artifact content is untrusted.
6. Test authentication, response limits, invalid proxy origins, timeouts, cancellation during every lifecycle phase, ambiguous creation and failed cleanup using local HTTP fixtures. Real-account verification remains separate and requires credentials entered in the application.

Implement the native client and durable cleanup record first, then provider settings, tool selection, approval presentation and live acceptance. This work does not narrow the existing Daytona requirement to configuration-only support.
