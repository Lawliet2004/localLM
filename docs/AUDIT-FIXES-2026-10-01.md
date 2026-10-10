# Audit follow-up — October 1, 2026

The user authorized fixes after the read-only audit and excluded Docker implementation. Changes were reviewed against a saved snapshot of the starting working tree, which already contained substantial uncommitted work. No commit, deployment, or dependency upgrade was performed.

## Defects addressed

| Finding | Result | Regression evidence |
|---|---|---|
| File edits truncated the original before writing succeeded | Stage, sync, recheck the expected hash, then replace through a pinned directory capability. Failed staging or replacement preserves the original. | Failed-write and locked-destination tests; hard-link test proves the original inode is not truncated; existing junction and path-boundary tests. |
| Opening a helper database connection interrupted live runs and replies | Startup recovery is explicit and called from application setup. Opening a second connection preserves active state. | Two-connection SQLite test; startup recovery and open-tail repair tests. |
| Forking used event count as message count and could include future output | Reconstruct messages from the selected event prefix. New response/end events include cumulative snapshots. Conversation, messages, events, preferences, and folder metadata commit together. | Early and completed fork boundaries; legacy history/run lineage; 5,001-event pagination; injected copy failure rolls back the conversation. |
| Successful turns left research tasks running; failed/denied operations appeared completed | Persist terminal task states with message/run completion. Distinguish completed, failed, skipped, and unknown operations. Pause preserves undispatched operations as pending. | Outcome tests, completed-result deduplication, bounded summaries, restart recovery, and injected completion failure rollback. |
| Resume could lose an empty task's identity or leave partially staged state | Reuse the staged task ID even with no operations; stage task and resume markers atomically. | Empty-ledger identity and failed resume checkpoint tests. |
| Context estimates omitted attachments and accepted stale async responses | Estimate the composed message, including attachment-only submissions. Invalidate on conversation, history, model/configuration, tools, and compaction changes; discard stale responses. | Attachment contents match submission; delayed prior-chat response is ignored; unchanged draft is re-estimated after history changes. |

## Application improvements

- Docker execution, daemon probes, and container routing were removed from the active tool path. Historical container selections block execution until the user explicitly saves local execution settings. They never silently run commands on the host.
- The model picker and readiness check follow the backend's loopback-only inference policy. URL parsing rejects deceptive hostnames, credentials, and unsupported schemes. Existing remote history stays readable.
- Cloud credential controls appear only for saved credentials or historical cleanup work. Cloud cleanup records and actions remain available.
- Completed chat turns are memoized. Activity events are assigned to turns without repeatedly filtering the complete event list for every turn. Polling preserves the cursor across generation changes.
- Trajectory loads pages of 200 events, exposes later fork points, and ignores responses from a previously selected chat.
- Models, Skills, Execution, and Connectors settings load on demand. Initial JavaScript fell from approximately 1.11 MB / 321 KB gzip to 1.03 MB / 300 KB gzip. A 200-turn regression confirms completed answer Markdown is not rendered again across 20 later streaming updates.
- CI now builds the complete web application, runs all script tests and Edge end-to-end tests, and uploads browser failure artifacts. Product, permission, security, and session documentation were aligned with current behavior.

## Verification

| Check | Result |
|---|---|
| `npm run typecheck` | Passed, including e2e TypeScript |
| `npm run lint` | Passed with warnings denied |
| `npm test` | 455 passed; one existing skipped test |
| `npm run test:scripts` | 39 passed |
| `npm run build` | Passed; bundled web worker unchanged from the starting snapshot |
| `npx playwright test` | 10 passed in Edge |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 407 passed; two existing ignored tests |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | Passed |

These checks exercise real SQLite transactions, temporary files, local HTTP/process fixtures, and Edge browser behavior. They do not establish live GPU inference, complete model downloads, account-specific connector compatibility, or release installer acceptance. The CI workflow was inspected locally; it was not dispatched remotely.

Vite still reports a main chunk above its 500 KB advisory threshold. Further splitting of inspector/terminal/highlighting code remains a performance opportunity. The warning threshold was not raised or suppressed.

## Separate future work

Docker stays deferred. Proposed new product features—reviewable edit sets with undo, a task recovery interface, memory controls, evidence inspection, readiness diagnostics, and benchmark tuning—were not added as part of these defect fixes. Broader native and release acceptance remains tracked in `SPEC.md` and the release documentation.
