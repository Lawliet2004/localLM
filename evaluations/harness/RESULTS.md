# Harness comparison — recorded evidence

Recorded 144 of 144 planned trials. **These measurements do not yet justify replacing the baseline.**

These candidate trials use the frozen V7 binary. Subsequent working-tree fixes have separate identities and are not credited to these trials.

Structural passes do not establish source accuracy, visual quality, or gameplay. Reviewed completion also requires a successful terminal run. Unfinished trials remain incomplete.

Latest frozen candidate: **v18**, with **4 of 72** remote/local trials recorded. Its evidence is separate from the earlier comparison.

| Build | Model pool | Workload | Trials / 12 | Structural passes | Reviewed completions | Reviews pending |
|---|---|---|---:|---:|---:|---:|
| baseline | remote | coding | 12 | 12 | 8 | 0 |
| baseline | remote | research | 12 | 0 | 0 | 0 |
| baseline | remote | document_visual | 12 | 12 | 10 | 0 |
| baseline | local | coding | 12 | 0 | 0 | 0 |
| baseline | local | research | 12 | 0 | 0 | 0 |
| baseline | local | document_visual | 12 | 0 | 0 | 0 |
| candidate | remote | coding | 12 | 11 | 9 | 0 |
| candidate | remote | research | 12 | 6 | 5 | 0 |
| candidate | remote | document_visual | 12 | 12 | 9 | 0 |
| candidate | local | coding | 12 | 0 | 0 | 0 |
| candidate | local | research | 12 | 0 | 0 | 0 |
| candidate | local | document_visual | 12 | 0 | 0 | 0 |

Later frozen revisions are separate from the 144-trial V7 comparison. Each revision/model/workload needs all 12 trials and their reviews before a full comparison is established.

| Revision | Pool | Workload | Trials / 12 | Structural passes | Reviewed completions | Reviews pending |
|---|---|---|---:|---:|---:|---:|
| v11 | remote | coding | 12 | 10 | 9 | 0 |
| v11 | remote | document_visual | 12 | 12 | 10 | 0 |
| v11 | remote | research | 12 | 10 | 9 | 0 |
| v12 | remote | research | 3 | 1 | 1 | 0 |
| v15 | local | coding | 2 | 0 | 0 | 0 |
| v15 | remote | coding | 4 | 4 | 2 | 1 |
| v15 | remote | research | 3 | 2 | 2 | 0 |
| v16 | local | coding | 2 | 0 | 0 | 0 |
| v16 | remote | coding | 4 | 4 | 1 | 0 |
| v17 | local | coding | 4 | 0 | 0 | 0 |
| v17 | remote | coding | 8 | 7 | 6 | 0 |
| v17 | remote | document_visual | 4 | 4 | 4 | 0 |
| v17 | remote | research | 4 | 2 | 1 | 0 |
| v18 | local | coding | 4 | 0 | 0 | 0 |
| v8 | remote | coding | 1 | 0 | 0 | 0 |
| v8 | remote | research | 2 | 1 | 1 | 0 |
| v9 | remote | research | 1 | 0 | 0 | 0 |

Per-trial revision records:

| Revision | Pool | Task | Trial | Structural pass | Reviewed completion |
|---|---|---|---:|---|---|
| v11 | remote | coding-csv | 1 | True | True |
| v11 | remote | coding-csv | 2 | True | True |
| v11 | remote | coding-csv | 3 | True | True |
| v11 | remote | coding-game | 1 | False | False |
| v11 | remote | coding-game | 2 | False | False |
| v11 | remote | coding-game | 3 | True | False |
| v11 | remote | coding-intervals | 1 | True | True |
| v11 | remote | coding-intervals | 2 | True | True |
| v11 | remote | coding-intervals | 3 | True | True |
| v11 | remote | coding-lru | 1 | True | True |
| v11 | remote | coding-lru | 2 | True | True |
| v11 | remote | coding-lru | 3 | True | True |
| v11 | remote | conversation-constraints | 1 | True | False |
| v11 | remote | conversation-constraints | 2 | True | True |
| v11 | remote | conversation-constraints | 3 | True | True |
| v11 | remote | document-brief | 1 | True | True |
| v11 | remote | document-brief | 2 | True | True |
| v11 | remote | document-brief | 3 | True | True |
| v11 | remote | document-data | 1 | True | False |
| v11 | remote | document-data | 2 | True | True |
| v11 | remote | document-data | 3 | True | True |
| v11 | remote | document-svg | 1 | True | True |
| v11 | remote | document-svg | 2 | True | True |
| v11 | remote | document-svg | 3 | True | True |
| v11 | remote | research-cache | 1 | True | True |
| v11 | remote | research-cache | 2 | True | True |
| v11 | remote | research-cache | 3 | True | True |
| v11 | remote | research-rust | 1 | True | True |
| v11 | remote | research-rust | 2 | True | True |
| v11 | remote | research-rust | 3 | True | True |
| v11 | remote | research-thinking | 1 | True | False |
| v11 | remote | research-thinking | 2 | False | False |
| v11 | remote | research-thinking | 3 | False | False |
| v11 | remote | research-wal | 1 | True | True |
| v11 | remote | research-wal | 2 | True | True |
| v11 | remote | research-wal | 3 | True | True |
| v12 | remote | research-thinking | 1 | False | False |
| v12 | remote | research-thinking | 2 | False | False |
| v12 | remote | research-thinking | 3 | True | True |
| v15 | local | coding-game | 1 | False | False |
| v15 | local | coding-intervals | 1 | False | False |
| v15 | remote | coding-csv | 1 | True | True |
| v15 | remote | coding-game | 1 | True | False |
| v15 | remote | coding-intervals | 1 | True | True |
| v15 | remote | coding-lru | 1 | True | False |
| v15 | remote | research-rust | 1 | True | True |
| v15 | remote | research-thinking | 1 | False | False |
| v15 | remote | research-wal | 1 | True | True |
| v16 | local | coding-game | 1 | False | False |
| v16 | local | coding-intervals | 1 | False | False |
| v16 | remote | coding-csv | 1 | True | False |
| v16 | remote | coding-game | 1 | True | False |
| v16 | remote | coding-intervals | 1 | True | True |
| v16 | remote | coding-lru | 1 | True | False |
| v17 | local | coding-csv | 1 | False | False |
| v17 | local | coding-game | 1 | False | False |
| v17 | local | coding-intervals | 1 | False | False |
| v17 | local | coding-lru | 1 | False | False |
| v17 | remote | coding-csv | 1 | True | True |
| v17 | remote | coding-csv | 2 | True | True |
| v17 | remote | coding-game | 1 | False | False |
| v17 | remote | coding-game | 2 | True | False |
| v17 | remote | coding-intervals | 1 | True | True |
| v17 | remote | coding-intervals | 2 | True | True |
| v17 | remote | coding-lru | 1 | True | True |
| v17 | remote | coding-lru | 2 | True | True |
| v17 | remote | conversation-constraints | 1 | True | True |
| v17 | remote | document-brief | 1 | True | True |
| v17 | remote | document-data | 1 | True | True |
| v17 | remote | document-svg | 1 | True | True |
| v17 | remote | research-cache | 1 | False | False |
| v17 | remote | research-rust | 1 | True | False |
| v17 | remote | research-thinking | 1 | False | False |
| v17 | remote | research-wal | 1 | True | True |
| v18 | local | coding-csv | 1 | False | False |
| v18 | local | coding-game | 1 | False | False |
| v18 | local | coding-intervals | 1 | False | False |
| v18 | local | coding-lru | 1 | False | False |
| v8 | remote | coding-game | 1 | False | False |
| v8 | remote | research-cache | 1 | True | True |
| v8 | remote | research-thinking | 1 | False | False |
| v9 | remote | research-thinking | 1 | False | False |

7 additional trial(s) have incomplete evidence, from a combined-cap stop or an explicitly recorded evaluation interruption. They are excluded from quality denominators; their artifacts and charges remain recorded.


Remote pool: configured DeepSeek-V4.1-Flash-1017 through River AI. Local pool: installed Gemma-4-E4B Q2_K_XL.

Each trial has a 600-second wall budget, 250,000-token allowance, 32,768-token context and 4,096 output tokens per request. The shared remote cap is 20,000,000 input-plus-output tokens, including excluded diagnostic attempts. Missing usage consumes the full reservation. Local tokens are counted separately.

- local: 9,331,026 charged/reserved tokens across 249 requests; 8 unsettled reservations conservatively charged.
- remote: 19,927,181 charged/reserved tokens across 1430 requests; 5 unsettled reservations conservatively charged.

Evaluation budget denials are explicit fixture stops and are not evidence of an upstream provider outage. False completion and factual-accuracy dimensions require individual review; pending counts remain explicit in comparison.json. A completed run with a failed deliverable is measured separately. Duplicate successful request signatures are a proxy for unnecessary calls, not proof that every repeat was unnecessary.


The Gemma runtime uses the recorded equivalent inline-loop adaptation of its installed unsupported template filter. Both builds use the same server and template. Trials are serialized on that server. Builds and tests overlapped some trials; latency is descriptive and is not a controlled performance result.

Evaluation workspaces are synthetic directories under the repository. Both builds therefore inherited the same ancestor AGENTS.md; captured request files record that context. Full Access commands have no OS filesystem sandbox. One baseline research trial copied two scratch files into the repository root; their tool-audit provenance and hashes are recorded in outside-workspace-changes.json and the files are preserved with that trial. Actual user conversation storage is read-only.

Excluded preliminary fixture/runtime attempts and the first diagnostic candidate are retained in the evaluation directory. Their remote charges remain within the shared cap. Source/visual reviews must record source URLs, factual omissions and limitations in review.json; an agent’s final paragraph is never sufficient evidence.

Reproduce/update: `python3 scripts/harness-evaluation-report.py --regrade`. Detailed measurements and exact build hashes: `.local/qa/harness-simplification/evaluations/comparison.json`.
