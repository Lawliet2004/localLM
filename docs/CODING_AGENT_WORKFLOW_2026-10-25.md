# Coding-agent workflow — 25 October 2026

This extends the existing LocalLM loop and conversation UI. It does not replace
the dispatch, authorization, native provider-history, or tool-envelope contracts.
Existing uncommitted changes were retained.

## Implemented behavior

- Live tool rows, channel buffers, completion updates and cleanup use the tuple
  `(conversationId, runId, stepId, providerCallId)`. Native audit/message IDs
  remain unchanged. Older unscoped transports retain their legacy fallback.
- Model deltas retain transient typed per-step rounds, including reasoning and
  text, rather than using only an undifferentiated assistant string. Duplicate
  positive channel sequences do not append text twice. Committed journal
  events replace provisional round projections; local channel and journal
  sequence counters are deliberately not compared as if they were one clock.
- Thought disclosures keep their step identity as the journal arrives. A user's
  disclosure choice and position while reading earlier thought text are retained.
  Conversation changes discard stale journal responses and notifications.
- One tool-outcome interpreter distinguishes successful, partial, failed,
  denied, cancelled, unknown, running and approval-pending work. Activity folding,
  tool status and file-change eligibility use that interpretation. Uncertain or
  partial mutations are not reported as successful file changes.
- Process completion now drains output and publishes terminal status before
  sealing a durable verification observation. The full suite exposed the former
  race: a sealed observation could coexist with a stale `running` check report.
  Reports may still show unverified work while observation sealing is pending.

No database migration, Tauri command, homepage control or user-facing replay
mode was added. Presentation remains Settings-only. Reasoning must be supplied
by the actual model; the runtime does not generate thought text for display.

## Reference evidence

The investigation used the linked
[REA repository](https://github.com/NabilRFG12/Reverse-Engineer-anything) and its
[process-capture workflow](https://github.com/NabilRFG12/Reverse-Engineer-anything/blob/main/docs/process-capture.md),
through the version-pinned `rea-agents@6.7.0` CLI. No MCP registration or analysis
engine installation was needed. Run the reproducible fixture with:

```sh
node scripts/coding-agent-reference.mjs
```

It launches installed OpenCode v2.0.32 with an isolated workspace, private app
configuration/data, and a loopback scripted provider. `env -i` prevents captured
OpenCode from inheriting provider credentials. REA captures terminal events,
process exit and selected file before/after evidence. Artifacts under
`.local/qa/coding-agent-reference/final/` record binary SHA-256, versions,
advertised tool schemas, provider requests, scenario and the complete capture.

The successful reference has seven model requests and six tool operations:
read, edit, shell, read, edit, shell. The first check exits 1 and the repaired
check exits 0, with failing/passing TAP output. The final file contains `fixed`.
An initial attempt revealed that this installed version uses `read.path` and
`shell`, rather than the previously documented `filePath` and `bash` names.
Those failed tool requests were visible in the trace and were not mistaken for
successful checks. The final fixture validates actual exit codes and output.
OpenCode reports both shell invocations as tool lifecycle `completed`; their
separate command exit metadata distinguishes failure from success.

[Claude Code's documented gather/action/verify loop](https://code.claude.com/docs/en/how-claude-code-works)
was a behavioral reference. Claude Code was not installed on PATH and was not
runtime-captured or decompiled. Scripted-provider evidence establishes harness
behavior, not the reliability or reasoning quality of a real model. REA capture
limitations and residual unknowns remain in its original evidence record; this
work makes no complete product-equivalence claim.

## Acceptance and verification

The request-aware Rust SSE fixture exercises `send_message_inner`, real schema
validation, file reads/edits, process launches, check reports, persistence and the
journal. It deliberately reuses one provider call ID across seven model steps.
It observes a failing report before repair, then requires a passing report before
the final response. Every emitted tool-stream event must carry run/step identity.

```sh
cd src-tauri
cargo test --lib
cargo test --lib chat::output_recovery_tests -- --ignored
cargo check --lib
```

The focused ignored module also covers malformed/repeated invalid calls,
bounded recovery, a model without exposed reasoning, and provider failure after
a successful write. These tests ran here only after a loopback-bind probe
confirmed this host permits TCP. No browser or provider secret was involved.

Frontend regression coverage includes reused call IDs with separate live output,
round overlays, repeated deltas, delayed journal reconciliation, nested legacy
outcomes, partial file changes and late responses after switching conversations.
Use `npx vitest run`, `npm run typecheck` and `npm run build` for the frontend.
The Impeccable detector returned no findings for the changed UI targets.

Native visual acceptance uses the bundled Tauri/WebKit application on an owned
X11/D-Bus display and isolated SQLite/app-data directories:

```sh
cargo build --manifest-path src-tauri/Cargo.toml --bin locallm --features tauri/custom-protocol
node scripts/native-coding-agent-acceptance.mjs
```

Set `LOCALLM_XVFB_BIN` when Xvfb is outside PATH. This authors a clearly named
persisted workflow fixture for rendering; actual tool execution is established
by the separate Rust test. Native capture results and any limitations are
recorded alongside the images under `.local/qa/coding-agent-native/`. No localhost
browser, alternate preview, proxy, browser automation or JavaScript injection
is part of that workflow. Capture review does not establish visual equivalence
to the two terminal applications.

### Recorded results

| Check | Result |
| --- | --- |
| Frontend suite | 891 passed, 1 skipped |
| Rust library suite | 1,018 passed, 24 ignored in the default run |
| Explicit loopback agent-loop acceptance | 7 passed, including the new complete coding workflow |
| Completion-observer regression | Passed after the ordering repair |
| Typecheck / production frontend build | Passed |
| Rust check / bundled native build | Passed; existing dead-code warnings remain |
| Impeccable UI detector | No findings |
| OpenCode REA reference | Complete; observed check exit codes 1 then 0 |
| Native rendering | Light/dark, 1440×1000 and 760×840; file diffs and restart restoration exercised |

The explicit loopback cases are among the 24 default ignored tests; the remaining
17 were not claimed as passing. No real local/API-model quality benchmark was
run. Native captures certify the authored rendering scenario, not real-provider
connectivity, comprehensive accessibility, or complete reference-product parity.

The first native capture batch completed all four theme/size combinations,
expanded the recorded diffs, and restored the conversation after restart; these
images were inspected together. A second, optional confirmation attempted an
additional scroll-to-beginning capture, but WebKit did not expose the user text
through the expected AT-SPI accessible name. That extra capture failed before
completing its batch and is not counted as passing. The unsupported optional
step was removed from the driver. No further visual capture rounds were run;
the passing first batch remains the visual evidence. Full-transcript top-of-feed
native capture and live-provider visual behavior are outside that evidence.
