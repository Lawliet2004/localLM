# Requirement-to-evidence matrix

Date: 2026-09-09. Scope: Windows x64, RTX 2050 4 GB, 16 GB RAM, MiniCPM5-2B
Q6_K, managed llama.cpp b10855.

`test-results/` is git-ignored, so JSON/PNG artifacts are local reproduction
outputs, not committed evidence. The matrix cites the reproducible script or
command plus the artifact name; `docs/PROGRESS.md` records the observed run.
Later progress entries supersede earlier ones.

Status key: **verified** (reproducible evidence), **partial** (implemented but
insufficiently verified), **missing** (not implemented), **external**
(needs credentials, authorization, or release infrastructure).

## 1. Chat, conversations, context

| Requirement | Status | Evidence |
| --- | --- | --- |
| Native lifecycle, window state, SQLite CRUD, streaming, cancellation, saved errors, search, rename/delete, copy/export, drafts, retry-as-new-turn | verified | `docs/DESKTOP.md`, `docs/EXPORTS.md`, `docs/CONTEXT.md`, `docs/PROGRESS.md` (native-smoke, retry, drafts, export slices) |
| Exact request context preflight + last-request indicator | verified | `docs/CONTEXT.md`, `docs/PROGRESS.md` (context + context-indicator slices) |
| Tool-result history across turns | verified | `docs/PROGRESS.md` (history slice), `cargo test history::` |
| Long-conversation compaction workflow | missing | No implementation; history is never truncated or summarized |
| Persisted usage/performance ledger | partial | Per-request indicator only; no durable ledger |
| Response-limit + durable error handling | verified | `docs/PROGRESS.md` (response-limit slice), `store::` reopen tests |

## 2. Permissions

| Requirement | Status | Evidence |
| --- | --- | --- |
| Ask / Auto-approve reads / Full access, Ask default, per-conversation persistence, Rust enforcement, durable audits, denial blocks turn, metadata cannot grant permissions | verified | `docs/PERMISSIONS.md`, `docs/PROGRESS.md` (permission-mode + skill-reader slices), `scripts/local-mcp-chat-smoke.mjs` → `local-mcp-chat-smoke.json` |
| Save/edit connector never launches; Full access limits | verified | `scripts/local-connector-smoke.mjs` → `local-connector-smoke.json`, `connectors::` CRUD tests |

## 3. Local MCP

| Requirement | Status | Evidence |
| --- | --- | --- |
| Encrypted config CRUD, explicit launch, discovery, selection, invocation, ownership, process-tree cleanup | verified | `scripts/local-connector-smoke.mjs` → `local-connector-smoke.json`, `cargo test local_mcp_` |
| MiniCPM allow/deny/auto/full-access chat workflow with identity, args, results, reload persistence | verified | `scripts/local-mcp-chat-smoke.mjs` → `local-mcp-chat-smoke.json` (re-verified 2026-09-09: ask-allow/deny, auto prompt, full-access no-prompt, reload persistence) |
| Cancellation, tool failure, server exit, disconnect durable outcomes | verified | `scripts/local-mcp-failure-smoke.mjs` → `local-mcp-failure-smoke.json` (2026-09-09) |
| Malformed/truncated input, unexpected exit, handshake/discovery failure, closed-transport selection, edit/disconnect race | verified | `connectors::tests::malformed_protocol_input_and_unexpected_exit_are_reported`, session snapshot changes |
| Structured arguments/environment, browsing, focus, validation, cancellation, duplicate names, pending states, secret visibility, stable IDs | verified | `scripts/local-connector-smoke.mjs` → `local-connector-smoke.json` (`structuredControls`, `secretHiddenByDefault`), `LocalConnectorForm.test.tsx` |
| Individual transport-frame size bound | partial | Local stdio frames capped at 4 MiB before SDK deserialization (`local_mcp_process::` frame tests); 512-tool/2 MiB accumulation still applies after page deserialization; remote HTTP framing unchanged |
| Shutdown observability | partial | `close()` reports protocol/signal/reap stages and failed reaps return stage detail; full-suite + native disconnect rerun deferred by disk pressure |
| Reconnect UX beyond disconnect-then-connect | partial | Covered by explicit disconnect/connect; no dedicated reconnect flow |

## 4. TrueForge remote connectors (14)

| Connector | Endpoint in repo | Auth | Fixture/live evidence |
| --- | --- | --- | --- |
| DeepWiki | `https://mcp.deepwiki.com/mcp` | none | verified discovery + real tool calls (`docs/PROGRESS.md` agent/permission-connector slices) |
| Linear, Notion, Sentry, Exa, Parallel Web, GitHub, Tavily, Bright Data, Supabase, Stripe, Confluence, Jira, PostHog | see `src/lib/catalog.json` | OAuth DCR or bearer | partial: presets, validation, discovery path, permission enforcement; **no live account coverage** |

Status: **partial**. Provenance: `catalog/PROVENANCE.md`
(truefoundry/trueforge `e956915`). Credential paths are now enforced in Rust:
OAuth DCR presets never read bearer tokens and reject token saves; bearer
presets (`github`, `tavily`, `bright-data`) require validated tokens with
redaction; public presets (`deepwiki`, `exa`, `parallel-web`) connect without
credentials (`connectors::tests::connector_lifecycle_errors_are_actionable_and_never_leak_tokens`).
Re-verified 2026-09-09: DeepWiki discovery + allow/deny/cancel tool flows
(`scripts/connector-smoke.mjs`, `scripts/agent-smoke.mjs`), remote permission
enforcement (`scripts/permissions-connector-smoke.mjs`). Missing per service:
sign-in/token storage/refresh/expiry/revocation/disconnect/reconnect
verification, exact per-conversation selection against live tools, actionable
account errors, real tool calls + continuation, credential redaction audit.
**External:** accounts, credentials, billing authorization, and official-docs
rechecks are required. Do not mutate external accounts for coverage without
authorization.

## 5. Skills (13)

| Requirement | Status | Evidence |
| --- | --- | --- |
| Pinned provenance, install/verify/inspect/activate/remove, reference reader with permissions | verified | `catalog/skills.lock.json`, `docs/SKILLS.md`, `docs/PROGRESS.md` (skill + skill-reader slices) |
| Dependency detection/status with actionable remedies | verified | `docs/SKILLS.md` (Dependencies), `skill_dependencies` IPC + Skills-page **Check dependencies**; covers all 13 skills for connectors/interpreters/external CLIs, never installs; native acceptance `scripts/skill-dependencies-smoke.mjs` → `skill-dependencies-smoke.json` + `skill-dependencies.png` (2026-09-09, IPC rows + rendered UI) |
| Explicit dependency installation, permission-controlled script workflows, versioned updates, artifact creation/opening, representative workflow per skill | missing | Reader returns text only; no execution, installation, update, or artifact workflows |
| Per-conversation activation | missing | Activation is global, captured per turn |

All 13 packages (`algorithmic-art`, `skill-creator`, `mcp-builder`,
`web-artifacts-builder`, `tavily-research`, `supabase`, `wiki-architect`,
`wiki-qa`, `linear`, `gh-fix-ci`, `notion-knowledge-capture`, `sentry`,
`jupyter-notebook`) share this status.

## 6. Workspace editing and artifacts

| Requirement | Status | Evidence |
| --- | --- | --- |
| list/read/create files + directories, traversal/junction protection, permission enforcement, audits | verified | `docs/PROGRESS.md` (workspace + permission slices), `workspace::` tests |
| Conflict-checked `edit_file` (fresh SHA-256, unique match, reliable write, durable audit) | verified | `scripts/workspace-edit-smoke.mjs` → `workspace-edit-smoke.json` (2026-09-09), `workspace::tests::edits_require_a_fresh_read_and_one_unique_match` |
| Reviewable diffs, file browsing UI, artifact previews/opening, untrusted HTML/script isolation | missing | No diff view, no artifact viewer, no preview sandbox |

## 7. Execution (local + Daytona)

| Requirement | Status | Evidence |
| --- | --- | --- |
| Local Python/Node/PowerShell with limits, approval, process-tree cleanup | verified | `docs/PROGRESS.md` (local-execution + permission slices), `execution::` tests |
| Daytona transport, durable ownership journal, cleanup recovery, credential settings, chat wiring, denial coverage | partial | `docs/DAYTONA.md`, `docs/PROGRESS.md` (Daytona slices), `daytona_*::` fixture tests |
| Daytona live execution, billing-authorized runs, cleanup after completion/cancel/failure/restart, rotation/expiry with pending cleanup, regional endpoints, artifact retrieval, ambiguous-outcome reporting, recovery-never-recreates proof | missing / **external** | No live account calls made; needs credentials + billing authorization |

## 8. Model/runtime management

| Requirement | Status | Evidence |
| --- | --- | --- |
| GPU detection, actual 43/43 offload reporting, load/unload/reload, CPU fallback, context/threads/layers/KV/Flash Attention/batch/generation controls, diagnostics | verified | `docs/RUNTIME.md`, `docs/PROGRESS.md` (telemetry, offload, diagnostics slices) |
| Pinned runtime/model download, hash verification, progress, cancellation, disk checks, inventory, startup cleanup of recognized partials | partial | `docs/PROGRESS.md` (download/install/inventory/recovery/cancel slices); current profile has no managed installs (only ~1.5 GiB free) and reuses `.local/` assets |
| Full model network-transfer acceptance, stalled/interrupted transfer, crash/restart recovery beyond fixtures, low-disk preservation, revalidation/removal, OOM recovery, RTX 2050 benchmark rates | missing | Not run in this cycle |

## 9. UI/UX

| Requirement | Status | Evidence |
| --- | --- | --- |
| Codex-inspired light/dark workspace, keyboard/focus/dialogs, responsive sizes, empty/loading/error/cancelled/disconnected states, readable tool/approval/code/table/artifact presentation | partial | `npx playwright test` (2 browser tests, screenshots to `test-results/*.png`); component tests only for newer dialogs; no systematic accessibility/responsive audit |

## 10. Security and reliability

| Requirement | Status | Evidence |
| --- | --- | --- |
| Vault encryption, IPC validation, CSP, traversal/junction/symlink handling, child-process ownership, MCP bounds, permission enforcement paths, crash recovery | partial | `docs/SECURITY.md`, `vault::`, `workspace::`, `skills::`, `execution::`, `local_mcp_process::` tests |
| Adversarial review: hostile servers/content, redirect/header leaks, OAuth mismatch, expired/revoked credentials, oversized catalogs/results, unavailable-tool selection, approval cancellation races, archive/symlink attacks, traversal, escape/orphans | partial | Unit/integration coverage exists; no end-to-end adversarial pass this cycle |
| Dependency vulnerabilities / third-party notices | partial | `npm audit --omit=dev` clean at installer time; no current license-notice bundle for app, runtime, model, skills |

## 11. Release

| Requirement | Status | Evidence |
| --- | --- | --- |
| Old unsigned NSIS installer + silent install/uninstall smoke | partial | `docs/RELEASE.md` (commit `8e36be7`, SHA-256 recorded); predates current code |
| Fresh optimized build, clean-profile onboarding, WebView2 prerequisites, install/launch/setup/inference/tools/shutdown/upgrade/rollback/uninstall, data preservation choices, interactive installer, signing, notices, provenance/hashes/benchmarks | missing / **external** | Needs release infrastructure + signing credentials |

## External dependencies still required

1. TrueForge account credentials for 13 authenticated connectors.
2. Daytona API key + explicit billing authorization for live runs.
3. Signing/release infrastructure for a shippable installer.
4. Disk capacity (~4 GiB) for managed runtime/model acceptance on a clean profile.
5. Time/account access for full adversarial, accessibility, and benchmark passes.
