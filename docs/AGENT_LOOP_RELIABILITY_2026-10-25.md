# Agent loop reliability — 25 October 2026

The diagnosed code paths have been repaired and exercised. This is not a promise
that arbitrary model output, provider requests, or external tools cannot fail.
Failures must remain visible, retain completed work, and avoid unsafe replay.

## Changes

- The frozen authorized tool catalog exposes the effective configured process
  timeout for process_start, check_run and verification_run. Runtime validation
  returns the field, numeric limits, and a no-side-effects rejection. The model
  cannot raise the configured limit.
- Process IDs and evidence IDs have distinct recovery instructions. Incorrect
  IDs do not launch checks. An invalid verification launch cannot create an
  evidence record pointing to a missing process.
- Check launch, pending work, failed/incomplete evidence, command-only success,
  and passing test reports remain separate. Later writes invalidate previously
  tracked check observations. These outcomes are journaled and displayed below
  the answer independently of the model's completion language.
- Repeated identical invalid requests are bounded. A failed provider request
  records its model round and preserves earlier tool results without silently
  replaying their actions.
- Timeline joins use run, step and call identity. Missing journal rows retain
  their timestamp positions rather than being matched by array index. Models
  without exposed reasoning do not acquire fabricated thought disclosures.
- File summaries consume native changes[] transactions, legacy single-file
  diffs, and conversation-owned full results restored from artifacts. File
  creation without a diff does not invent line totals. Failed, rolled-back and
  uncertain mutations do not become successful edit summaries.
- Conversation CSS now uses one current surface specification, shared palette
  tokens, consistent row sizing, and expanded groups/diffs without tree rails.
  See ACTIVITY_DESIGN.md. Presentation preferences remain Settings-only.

## Verification

| Check | Result |
|---|---|
| Rust library suite, cargo test --lib | 1,018 passed, 0 failed; 22 ignored in that default run |
| Final explicit agent-loop acceptance | 6 passed, 0 failed |
| Frontend suite | 873 passed, 1 skipped |
| Typecheck | Passed |
| Production frontend build | Passed |
| Rust library check | Passed |
| Desktop executable build | Passed |
| Mechanical UI detector | No findings |

The explicit acceptance command is:

```sh
cd src-tauri
cargo test --lib chat::output_recovery_tests -- --ignored
```

It uses isolated temporary app data and a scripted loopback SSE provider. It
exercises an ordinary question without reasoning, the effective timeout schema,
repeated invalid calls, exhausted output recovery, successful file creation after
truncated output, and provider failure after a successful write. The last case
checks that the file and journal survive and the write is not repeated. These
tests are now included in the backend CI workflow. The final sixth case was
added after the default suite run and passed explicitly; the other unselected
ignored tests remain unverified. No real provider credentials were used.

Two stale assertions discovered during verification were corrected: the nudge
now describes incomplete checks, and the remote recovery fixture must expect the
configured provider output limit rather than the local token preference.

## Remaining acceptance boundaries

- No live DeepSeek/local-model reliability measurement was performed. Provider
  availability, network timeouts, model mistakes and task-specific failures can
  still occur; the new handling makes those outcomes explicit.
- Exact visual/reference matching remains unverified. No localhost browser
  preview or alternate preview route was used. Native screenshot comparison is
  still required before claiming visual acceptance.
- Use a build containing these changes for the next app session. Source checks
  do not update an independently installed copy of LocalLM.
