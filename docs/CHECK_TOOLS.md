# Structured build and test reports

For checks tied to selected source hashes, use `verification_run`,
`verification_report`, and `verification_compare`. These wrap the same process
and report paths, retain immutable evidence, and distinguish historical passing
tests from results whose selected source/configuration still matches. See
[source-bound verification](DEVELOPMENT_TOOLS.md#3-source-bound-verification).

`check_run` starts a shell command through the existing conversation/workspace-owned process registry. `check_report` reads its retained output without running the command again. Both use the native `AgentTool` registration, strict object schemas, execution-source selection and capability checks. Launching uses configured shell enablement, working directory, environment, timeout, memory policy and approval settings. This is process management, not operating-system isolation.

Code, Creator, Coding and Standard profiles include these tools when the execution source and shell are available. Minimal keeps its existing narrow catalog. Reporting an already owned job remains available when starting new shell work is disabled. Explicit capability kills still reject stale calls.

Example call:

```json
{"name":"check_run","arguments":{"command":"cargo test --lib","format":"cargo","yield_ms":100,"max_cases":0}}
```

The returned `processId` is used with `check_report`, `process_poll`, `process_write` and `process_stop`. The native payload (inside `data` after result normalization) includes:

```json
{
  "processId": "proc-<host-generated-uuid>",
  "started": true,
  "process": {"state": "running", "exitCode": null},
  "report": {"verdict": "running", "testsPassed": null},
  "isError": false
}
```

This is an illustrative shape: actual responses contain additional metadata. `started:true` means the command launched; it never asserts that tests passed. A report can detect test failures even when a runner exits zero. A successful exit with no recognized report yields `command_succeeded`, unknown test success and null counts. Zero tests or entirely skipped/todo suites, partial logs, invalid reports, timeouts, cancellation and unknown outcomes have separate verdicts. Commands may change files; failed or uncertain commands are never automatically replayed.

Supported output includes Cargo/libtest suite summaries and case diagnostics, Vitest/Jest JSON, Vitest text summaries, top-level TAP, and JSON with `schemaVersion:"locallm.check-report/1"`. The generic JSON format has `counts:{total,passed,failed,skipped,todo}` and optional `cases:[{name,outcome,durationMs,diagnostic}]` and `runnerFailed`. Counts must agree, be nonnegative integers and total at most one million. Case outcomes cannot exceed declared counts. Summary-only reports remain useful; absence of individual cases is explicit.

Case details page with `case_offset` and `max_cases` (0–200, default50), failing cases first and source order within an outcome. `sourceIndex` preserves original position. Pin `expected_sha256` from `report.sourceSha256` when paging; the hash includes requested parser format and both output streams. Changed output or parser format rejects the pin. `caseDetailsComplete`, `availableCases`, `hasMoreCases` and `nextCaseOffset` distinguish missing details from later pages. Names and diagnostics have explicit clipping flags.

The process registry retains256KiB of output. Loss is reported as `incomplete:true` and `countsScope:"retained_output"`; retained passing summaries cannot prove a full pass. Mixed reports or streams warn that selected counts cover only part of the command. ANSI formatting is removed for parsing. Parsing never grants permission to execute another command. Real terminal output remains untrusted evidence.

Large model-visible results are saved as artifacts. Native process handles survive bounding exactly; compact check verdict/count/hash/cursor fields are retained where the serialized result budget permits. Case diagnostics can be read from the saved artifact or requested again with a smaller detail page. Re-reading a report never reruns the check.

Verification is tracked in `INFERENCE_TOOL_IMPLEMENTATION_PLAN.md`. Windows process and parser behavior require Windows runtime verification; Linux fixtures do not establish that result.
