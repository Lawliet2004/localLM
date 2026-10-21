# Full implementation checklist — 2026-10-16

This closes the implementation audit against the attached full-stack request,
not just the earlier defect-remediation list. Existing user changes and data
were preserved. Commands, measurements, boundaries and setup are in
[the implementation report](IMPLEMENTATION_REPORT.md).

| Requirement | Delivered and verified evidence |
| --- | --- |
| 0. Audit and baseline | Inspected React → Tauri → native/worker → llama.cpp flows, settings and existing changes. Fresh baseline: 490 frontend passes/1 skip; 411 native passes/5 ignored. Reproduced descendant-process leakage and repeated Markdown rendering before repairing them. |
| 1. Research | Reused SearXNG/evidence pipeline; health/failover, modes, structured JSON/CSV/TSV, bounded persistent caches and clear-cache. Transport/render: 19 passes; worker/storage: 9 passes; grounding smoke passed. Citation, SSRF and cancellation regressions pass. |
| 2. Bash and PTY | Real configurable Bash/PowerShell, canonical cwd, structured results/artifacts, UTF-8 streaming, stdin, limits, cancellation/process-tree cleanup; xterm acknowledgement backpressure. Real native shell and PTY regressions pass. |
| 3. Python | Files/snippets, frozen approved source, existing venv, imports, arguments, stdin/env, tracebacks, limits and previews. Tests execute installed Python, a real venv and generated artifacts. |
| 4. Weather | Validated `get_weather`, current/hourly/daily, geocoding, C/F, timezone/freshness, persistent cache and result component. Provider/worker/component tests and live Tokyo request passed. |
| 5. Orchestration | Offline JSON Schema validation, AREX normalization, policy across foreground/background/PTC, approval cleanup, persisted failures, conditional exposure and no duplicate executor. Native regressions pass; installed Gemma produced a valid structured diagnostic call. |
| 6. Runtime/performance | Lifecycle/cache/budgeting repairs retained; real GGUF validation and capability retest added. Installed CUDA runtime, 4-GiB GPU, CPU fallback, cold/warm streams, cache and cleanup measured. Completed Markdown parsing reduced from 80 repeated renders to zero in a controlled regression. |
| 7. Metrics | Native timings, monotonic HTTP-client first token, per-round cancellation/error handling, per-message persistence/forks and GEN/PROMPT/TTFS UI. Known-input and persistence tests pass. |
| 8. Automation | Separate disabled-by-default Browser/Computer modules. Real Rust → Playwright MCP → Chrome acceptance, isolated X11 input/capture, capability gating and cancellation passed. |
| 9. Git | Native literal-path CLI; status/diffs, stage/unstage, staged-tree-bound confirmed commit, branches and history. Disposable real repositories cover normal, unborn, stale-review and error paths. |
| 10. Settings/UX | All requested destinations inherit the existing design system and use real backend detect/test/save states. 34 Playwright tests pass; desktop/narrow light/dark and genuine native screenshots inspected. Design detector: no findings. |
| 11. Final QA | 509 frontend passes/1 skip; 432 native passes/5 ignored; typecheck, build, strict Clippy and 34 Playwright tests pass. Research/transport/native integrations and actual app smoke passed. Secret Service persistence verified across processes, temporary entry deleted. |

Opt-in skips, untested operating systems/services and remaining release work are
listed in the report. Windows/macOS, authenticated-account and installer
acceptance are not inferred from Linux or fixture tests.
