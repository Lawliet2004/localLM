# AREX research tools

Research mode offers `search`, `visit`, `update_context`, and `finish` alongside its existing web tools, using the argument names in [BAAI's published tool definitions](https://huggingface.co/BAAI/AREX-Turbo/blob/main/inference/prompts.py). Standard and other web-enabled modes switch search/fetch tools to these contracts when the selected model name/path contains `AREX`. Chat and Minimal modes do not gain web access. Existing conversations refresh their tool catalog automatically.

- `search({query: string[]})`: 1–4 queries, up to ten results per query before fusion. Uses the configured search provider and fallback without a nested model planning/synthesis call.
- `visit({url: string | string[], goal: string})`: 1–4 HTTP(S) URLs, using the existing protected fetch/extraction worker. Returns bounded text and per-page failures; the calling model evaluates the text against the goal. It does not automatically verify or summarize the page.
- `update_context({context: string})`: up to 16,000 bytes of model-authored notes with facts, URLs, uncertainties, rejected paths, and next steps. Archives older model/tool exchanges and keeps user instructions plus the current tool exchange intact. Transcript/audit rows remain available. Automatic compaction preserves the latest live checkpoint.
- `finish({answer: string, evidences: [{evidence: string, url: string}], confidence: string})`: returns the supplied answer and supporting source links directly and ends the turn. Confidence accepts a score from `0%` to `100%`; it is model-reported, not an independent verification score. Empty evidence is allowed for an honest unresolved answer.

`finish` and `update_context` must each be the only call in their batch. Normal permissions, cancellation, result bounding, and auditing still apply. The research guard counts `search`, `visit`, and context updates; rewriting context does not count as new evidence. The guard warns after two unproductive calls, finalizes after four, and has a 24-call backstop (batches can contain up to four queries/URLs).

This is an adaptation, not a reproduction of BAAI's evaluation environment. It uses LocalLM's structured tool-call transport; the model runtime must support the model's tool-call template/parser. BAAI's raw XML-only inference example is not a separate execution protocol here. The benchmark instruction that every question is guaranteed to have an answer is deliberately not used.

Foreground child agents support these control tools. Background children support the store-only control tools, but web search/visits still require the foreground dispatcher, like the existing web tools.

Validation: `cargo test --manifest-path src-tauri/Cargo.toml --lib`, `npx vitest run src/web/arex.test.ts src/web/phase1.test.ts`, and `npm run build`. Live model quality must be checked separately against the chosen GGUF and runtime.
