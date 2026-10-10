# Agent evidence checks

The agent checks important factual premises and material answer claims against
retrieved evidence. Current, uncertain, snippet-only, or conflicting claims
should use primary sources and corroboration from another source when available.
The shared instructions apply to ordinary chat and child agents.

## Bounded completion loop

Ordinary research replies and AREX `finish` replies receive a deterministic
evidence check before completion. An unsupported excerpt, a source disagreement,
or reported confidence below 80% permits one correction pass. The correction
pass allows at most two additional tool calls, including cached calls, and then
requires a final response. It preserves the existing permissions, offline mode,
cancellation, stagnation guards, and overall turn budgets.

The evidence check uses recorded tool evidence rather than model-provided
citations. It normalizes citation formatting and compares directly conflicting
quantity/polarity assertions before checking answer excerpts. Research drafts
after a web tool call are buffered until accepted; the saved final response is
loaded through the existing chat completion flow. Verification failures and
unresolved support are disclosed rather than silently treated as success.

The initial current-fact shortcut uses the staged worker's verification report,
avoiding another model round or duplicate audit. Source URLs still pass through
the shared citation grounding check.

## Staged research

- Read up to three independent pages concurrently, retaining ranked source
  order and caching each URL within the request.
- Retain matching excerpts from distinct source URLs so corroboration is not
  discarded as duplicate text.
- Derive evidence quality from whether the fact appears in retrieved page text
  and how many source-domain groups contain it. Repeated URLs and pages from
  the same domain group do not increase confidence.
- Schedule at most one additional targeted query for snippet-only or conflicting
  evidence, subject to remaining query allowance and deadline.
- Preserve conflicting variants and disclose missing corroboration. Render a
  single excerpt or benchmark table directly instead of paying for redundant
  model synthesis.

## Limits

These checks establish evidence grounding, not guaranteed factual truth.
Exact excerpt matching can miss valid paraphrases, and corroborating publishers
can repeat the same upstream mistake. Conflict detection deliberately handles
closely matching assertions; it is not a general semantic contradiction solver.
Confidence values describe evidence quality, not calibrated probabilities.
Source-domain grouping is conservative and does not prove publisher independence.
When sources are unavailable or budgets expire, the agent should name the
unresolved claim instead of inventing a verified result.

## Regression checks

```text
npm run typecheck
npm run lint
npm test
npm run build:web
node --test scripts/agent-verification.test.mjs scripts/staged-worker.test.mjs
cargo test --lib --no-default-features  (from src-tauri)
```

Fixtures exercise conflicting quantities, citation-only formatting, distinct
product identities, source corroboration, concurrent fetch ordering, low
confidence, and the two-call correction limit. They use recorded/local fixtures;
they do not establish accuracy or latency for every installed model.

## Evidence-backed completion (records, not claims)

A conversation's completion state is computed from stored verification
records — criteria plus the checks that actually ran — never from the model's
own "done" wording. The Evidence block under the latest answer shows the
rolled-up state and stays collapsed by default.

- **Criteria** are checkable conditions (`task_criteria`). Users add or edit
  them inline; the agent may propose criteria with `propose_criteria`
  (allowed in Plan and Act) and they are labeled "agent" — a proposed
  criterion is a review item, not self-certification.
- **Checks** (`verify_run`, or the "Run check" input, `verification_run_user`)
  execute a shell command in the conversation's resolved workspace — the
  isolated task root while a task workspace is active — with the same
  approval class, clean environment, and kill policy as local execution, a
  10-minute timeout, full output saved to an artifact, and an 8 KiB tail
  excerpt. Exit code 0 records `passed`, a nonzero code `failed`, and a
  launch failure or timeout `unavailable`.
- **Research checks**: a `web_search` whose claim verifier ran stores a
  `research` record with the claim→source mappings. It reports `passed` only
  when every claim was certified, else `inconclusive`, with the limitation
  "Citation match is heuristic; it does not prove the claim."
- **Calculations**: `run_code` with a `criterion_id` stores a `calculation`
  record keyed on the exit code.
- **Fingerprints**: each workspace-backed record hashes the tree state it ran
  against (temp-index `git write-tree` for repos — the real index is never
  touched — or a bounded sorted path/size/mtime walk for non-git roots). When
  the tree changes, the record reads as `stale`; staleness is computed at
  read time and never stored.
- **Completion state**: no records → `work-completed` (explicitly
  unverified); any non-stale `failed` → `checks-failed`; every criterion
  covered by a fresh pass with all records green → `checks-passed`; stale or
  `unavailable`/`inconclusive` records → `needs-review`; otherwise
  `partially-verified`.

Limits: a green exit code is evidence, not proof — the record stores what
ran, not a judgment that the criterion is truly met. `verification_record_output`
serves the saved log bounded at 2 MiB with a truncation note.
