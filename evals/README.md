# Agent evals

Task suite for `scripts/eval/run.mjs` (docs/EXTENSIONS.md §2.5). Each task
runs through the real agent loop of a running debug app: real model, real
tools, real approval path and `session_events` log. Nothing is simulated.

## Running

1. Launch the debug app with WebView2 CDP (same setup as the smoke scripts):
   `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223`.
2. Load the model to measure in **Models & runtime**. For a remote model,
   select it as the preferred model.
3. `npm run eval -- --presets standard,coding --repeats 3 --seed 1234`

Reports go to `test-results/eval/eval-<timestamp>.{json,md}`. To compare
models, run once per model and merge the reports:
`node scripts/eval/merge.mjs test-results/eval/eval-A.json test-results/eval/eval-B.json`.

The runner sets `sampling.seed` for the run, points the workspace at a fresh
temp copy of each fixture, and restores your preferences and workspace when it
finishes. It keeps its conversations (titled `[eval] …`) for inspection unless
you pass `--cleanup`. A seed makes only local llama.cpp sampling reproducible;
remote providers drop the seed or treat it as best-effort.

## Metrics

| Metric | Source |
|---|---|
| pass | all task assertions hold |
| tool-call validity | `tool_call` events ÷ (those + a turn ended by a malformed call) — `n/a` when no call was attempted |
| tool calls / errors / denials | `tool_call` and `tool_result` events (envelope `status` or legacy `isError`) |
| approvals | approval requests the runner answered |
| peak input tokens | runtime-measured prompt size from the turn's context events |
| wall time | measured by the runner |
| output tokens, cache hit % | llama-server `timings` events (`predicted_n`, `cache_n` / `prompt_n`) — n/a for providers that report none |
| KV restore | outcome of the turn's `kv_slot` restore event, if any |

Every metric is reported as mean ± sample SD over repeats.

## Task format

`evals/tasks/<id>.json` (the filename must equal the id):

```json
{
  "id": "fix-add-bug",
  "fixture": "buggy-math",
  "presets": ["standard", "coding"],
  "prompt": "…",
  "approve": ["read_file", "edit_file"],
  "timeout": 600,
  "assertions": [{ "type": "fileIncludes", "path": "src/math.js", "text": "return a + b;" }]
}
```

- `fixture`: a directory under `evals/fixtures/`, copied fresh for each run.
  Leave it out for an empty workspace.
- `presets`: optional allow-list of presets. Other combinations are reported
  as skipped.
- `approve`: `"all"`, `"none"` (the default), or a list of tool names to
  allow. Every other approval is denied, and `ask_user` is always dismissed.
- Assertion types:
  - Answer checks: `answerIncludes` (`text`, a string or list), `answerMatches` (`pattern`, `flags`). Both accept `negate: true`.
  - Workspace files, with paths relative to the workspace: `fileIncludes`, `fileExists`, `fileAbsent`.
  - Tool use: `toolCalled` (`name`, `min`), `toolNotCalled`, `maxToolCalls` (`max`).
  - Turn outcome: `status` (`equals`: `complete`, `error`, `interrupted`, `timeout`).

`npm run test:eval` checks the scorer and validates every task file without
needing the app.
