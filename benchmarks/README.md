# Benchmarks

## Static gates (CI, no model)

- `node scripts/dump-config-smoke.mjs` and friends: seam presence.
- `node benchmarks/minimal-bench.mjs`: Minimal preset surface.

## Live gates (credentialed: loaded local model or provider key)

`benchmarks/tasks.json` defines three fixed tasks (self-correction loop,
bounded codegen, repo comprehension). Run each task once per runtime mode
(Standard / Minimal) and per provider (local, DeepSeek, Claude, GPT) and
record Pass@1 plus cost. No harness changes are needed to run them: drive via
the chat UI, the Python SDK, or the headless CLI against a scheduled task.

Rules: real credentials only; never claim fixture runs prove provider
accounts work. Prefix-cache behaviour is observable via the preflight token
counts in the Trajectory (`context_injection` + generation events).

DSBench / LM-Eval harness adapters are future work; these tasks are the
standing Minimal-mode quality gate until then.
