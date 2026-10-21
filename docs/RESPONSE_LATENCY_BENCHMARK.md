# Response latency diagnostic

Run `python3 scripts/benchmark-response-latency.py --help` for explicit local
runtime and GGUF options. No settings database, saved credential, remote API,
package install or generated code execution is used. The runner starts a
loopback-only authenticated server with an ephemeral key, and stops its process
group on success, failure or interruption. CPU (two threads) is the default.
The installed llama.cpp b11669 runtime is the acceptance target.

Example (replace paths with your own):

```sh
python3 scripts/benchmark-response-latency.py \
  --runtime /absolute/path/to/llama-server \
  --model /absolute/path/to/model.gguf \
  --output .local/response-diagnostic-run-1 \
  --samples 3 --modes auto quick
```

Each mode runs simple arithmetic, Python function generation and grounded
research fixtures with one synthetic tool schema on/off. All tasks include
their required information in the prompt; the tool, when selected, returns
only the declared fixture. Research uses fixed excerpts and citation checks;
it does **not** test live search quality or search-service latency. Python
correctness uses a restrictive AST check and never executes model output.

For each sample the runner requests an uncached prompt, primes an identical
cache-enabled request, then measures an identical warm request. Priming is
stored but excluded from summary statistics. Inspect per-round backend cache
counters to confirm actual reuse; “cold” is `cache_prompt=false`, not cold disk,
a fresh process, or flushed operating-system caches. Tool schemas stay present
throughout a multi-round case, including its final round.

`measurements.json` records configuration, runtime version/model paths, runner
fingerprint, platform/CPU metadata, model file size/modification time, template
capabilities, effective thinking-toggle detection, unsupported modes, individual
SSE rounds, correctness, first streamed token, first non-reasoning answer
content, total time, and process cleanup. Summary groups contain sample counts,
correctness rate, median and nearest-rank p95. Failed/incomplete cases lower
correctness and are excluded from completed latency aggregates; missing timing
remains null. Incorrect completed answers still appear in latency statistics,
alongside their correctness result. Exit status is nonzero if a case fails,
an answer fails its fixture check, or a requested mode is unsupported.

Thinking support is checked through the runtime's effective `/apply-template`
output and `/props`; an absent embedded GGUF template does not imply an absent
runtime capability. This is a standalone backend diagnostic. App preparation,
approval, real tools/search and UI rendering need separate integrated checks.
Small samples and descriptive timing differences do not establish a statistical
or causal speedup. Use the same model, runtime, hardware, configuration and
fixture matrix for before/after comparisons, with correctness held visible.

Reports go to a new output directory to preserve previous evidence. The report
contains synthetic prompt/answer text and local paths; it contains no ephemeral
server key. The runtime log is retained next to it for failure diagnosis.

Focused verification:

```sh
python3 scripts/benchmark-response-latency.test.py
```
