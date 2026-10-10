# Small-model web research recovery

## Observed failure

The September 29, 2026 benchmark request asked for a chart of ten coding and knowledge-work benchmarks comparing GPT-6 Astra and Sonnet 5.5.

The saved research session `sess-2fe0264c-a2af-4bcf-8b31-73eb7f3987e3` searched only for Astra, recorded `fetch failed`, and returned no evidence. The conversation WAL also contained the subsequent Sonnet call receiving the same cached failure with `repeated: true`. A failed request had disabled searches for the rest of that run. The old error did not retain the underlying connection diagnostic, so it cannot establish why that first connection failed.

The separate research database was readable. The main conversation database reported malformed SQLite data; investigation used a copy and WAL text without changing or repairing the user's database.

## Changes

- Failed searches are not cached as successful responses or as a run-wide provider outage. Later independent queries can recover.
- Each configured SearXNG endpoint receives part of the timeout budget. Errors retain the endpoint origin and underlying connection codes.
- Explicit comparison names take precedence over phrases such as “these two models.” Query planning uses short subject-specific searches and preserves both subjects.
- The user's original comparison and requested row count travel separately from an agent's abbreviated keyword query.
- Each initial query can read three new pages. One query cannot consume another query's allowance. Up to four targeted follow-ups use benchmark names already found in sources; each can read one additional page.
- Structured table cells and labelled chart blocks are copied directly, including units and evaluation notes. They do not depend on a small model rewriting the numbers.
- Prose extraction accepts partial evidence. Explicit model/benchmark/score assertions can become rows. Pronouns require an unambiguous source title; other prose remains attributed separately.
- The answer preserves partial results, marks missing cells, and keeps different versions, units, conditions, and conflicting scores separate. It does not invent ten complete pairs or a ranking of benchmark importance.
- Tool-output compaction prioritizes the retrieved evidence over duplicated document payloads. Oversized output remains available through the saved artifact.

## Verification

Regression tests cover the original request, ten-row retention, mixed coding and knowledge-work metrics, exact model versions, separate-column attribution, table conditions, blocked pages, independent-query recovery, targeted follow-ups, and prose/table preservation. Provider tests cover connection diagnostics and endpoint timeout failover. Rust tests cover failed-cache recovery and research evidence handoff.

A live replay used the installed MiniCPM5-2B Q6_K model with an isolated llama.cpp server and the bundled worker. It completed in about 90 seconds: nine searches, ten readable pages, three failed page reads, and 21 accepted evidence items. Four searches targeted missing Astra scores using discovered benchmark names. It produced a cited partial table containing scores for both models; it did not verify ten directly comparable pairs. Live results varied across runs, including blocked pages and different search hits.

The live replay exercises the worker and local model. It does not establish that the complete desktop conversation succeeded. The malformed conversation database prevented normal inspection of the full saved chat. No database repair or desktop restart was performed.

Run checks with:

```text
npm run typecheck
npm run lint
npm test
npm run build
node --test scripts/bundled-worker.test.mjs scripts/staged-worker.test.mjs scripts/web-transport.test.mjs scripts/web-render.test.mjs
cargo test --lib --manifest-path src-tauri/Cargo.toml
cargo build --manifest-path src-tauri/Cargo.toml
```

`npm run build` regenerates the embedded worker. Rust must then be rebuilt, and the harness relaunched, to load the changed worker and cache behavior.
