# Harness implementation evaluation

`tasks.json` contains twelve synthetic outcome tasks, with two known and two held-out cases for each workload. `regressions.json` maps sanitized historical failure patterns to production-path tests. No actual conversation is replayed or modified.

Run the production headless entry point with `cargo build --features harness-eval --bin locallm`. The evaluator creates a separate app database, workspace and permissions record for every trial. Its inference proxy records requests, responses, provider usage and conservative token reservations. It never saves provider credentials.

The baseline is the initial dirty source archive, with only the same headless entry point added. Both evaluator builds use the debug profile. Their binary SHA-256 and source manifests are recorded under `.local/qa/harness-simplification`. The installed application is not replaced by an evaluation run.

Run all three trials with:

```sh
python3 scripts/harness-evaluation.py --build baseline --pool remote
python3 scripts/harness-evaluation.py --build candidate --pool remote
python3 scripts/harness-evaluation.py --build baseline --pool local --local-url http://127.0.0.1:18492/v1
python3 scripts/harness-evaluation.py --build candidate --pool local --local-url http://127.0.0.1:18492/v1
python3 scripts/harness-evaluation-report.py --regrade
```

Local trials must run serially against the same server configuration. Do not reset the shared token ledger when retrying an evaluation. The 20-million-token remote cap includes preliminary and excluded attempts. A per-trial allowance of 250,000 tokens prevents one stalled task from consuming the comparison budget. Missing usage is charged at the full reserved amount. Do not call a token-limited trial a provider reliability failure without inspecting the ledger.

The proxy records each rejected reservation in `budget-denials.jsonl`, distinguishing a per-trial allowance from the combined remote cap. A combined-cap rejection stops the matrix and marks that trial as incomplete evidence; the report excludes it from quality denominators while retaining its artifacts and charges. Each new `fixture.json` also records the evaluator script and task-definition SHA-256 values so changes to accounting or grading can be traced independently of the application build.

Run `python3 scripts/harness-evaluation-accounting.test.py` to check reservation concurrency and settlement against temporary ledgers. Negative, fractional, boolean or absent usage values consume the full reservation. These tests never write to the real benchmark ledger. The report names the latest frozen candidate and its own trial count separately from the completed historical V7 matrix; an older comparison does not certify a newer build.

Objective graders execute resulting code and inspect saved files. They do not accept the assistant’s self-assessment. A structural pass for a research answer, SVG or game is provisional until a `review.json` records source accuracy, visual inspection or actual interactive behavior. Browser-preview workarounds are not part of this workflow. Reports keep ungraded dimensions explicit.

Keep each candidate binary and its source archive immutable. The initial comparison uses V7; later working-tree repairs must receive a new build identity and cannot retroactively turn a V7 failure into a pass. `harness-source-reduction.py` reads the original dirty baseline archive and reports production and test lines/bytes separately. Its Rust test classification is lexical, so the numbers are estimates rather than compiler coverage.

Freeze a later evaluator with `python3 scripts/harness-freeze.py --revision v9` (choose a new revision if that one exists). It builds before copying and checks input hashes across compilation and archiving. Its manifest includes the binary, source inventory and archive hashes.

Use a separate revision for follow-ups, for example `python3 scripts/harness-evaluation.py --build candidate --revision v8 --pool remote --tasks coding-game,research-thinking,research-cache --trials 1`. Place that frozen binary and manifest in `.local/qa/harness-simplification/candidate-v8/`. These trials share the same global ledger and appear separately from the primary comparison.

The installed Gemma template uses a filter unsupported by its installed runtime. Evaluation uses an equivalent recorded inline-loop template on the same server for both builds. The proxy forwards local cancellation by closing the upstream connection; otherwise a cancelled generation keeps occupying the single inference slot. Preliminary attempts made before these fixture repairs are excluded from outcome comparisons, retained for audit, and remain charged in the ledger. Reclassifying a diagnostic trial name never changes its reservations or charges.

Review all saved claims, including optional commentary in deliverables. Record factual mistakes even when the requested arithmetic or schedule is structurally correct. Game state assertions with stubbed canvas/clocks establish logic behavior only; they do not establish browser rendering, real-time playability, or accessibility. Evaluation budget denial is distinct from a provider outage. Latency measurements are descriptive while builds and tests share the host.

Research reviews use the official [DeepSeek thinking-mode contract](https://api-docs.deepseek.com/guides/thinking_mode/), [Rust JoinHandle documentation](https://doc.rust-lang.org/std/thread/struct.JoinHandle.html), [SQLite isolation documentation](https://sqlite.org/isolation.html) and [MDN Cache-Control reference](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Cache-Control). Record the retrieval date: provider contracts can change. River AI behavior requires its own observed evidence rather than inference from the model name.

`RESULTS.md` and the detailed local `comparison.json` report only completed recorded trials. Baseline replacement requires all deterministic checks, fewer harness-caused failures, improved aggregate completion and no workload regression. Incomplete or unreviewed live trials do not satisfy that gate.

The synthetic workspace is not an OS sandbox for Full Access commands. Both builds inherit the repository's ancestor instructions. One baseline research trial copied two scratch files into the repository root; they were attributed to its saved command audit, hashed and moved into that trial's `outside-workspace/` evidence directory. `outside-workspace-changes.json` records their original paths. This containment limitation must be accounted for in future fixture design.

The brief structural grader accepts equivalent uncertainty wording instead of requiring the literal word “undecided”; this correction applies uniformly during `--regrade` to all builds. Full factual review still checks the launch date, approval status, owners and decision. Unknown task ids now fail before provider requests rather than silently omitting cases.
