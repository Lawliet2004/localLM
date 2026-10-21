# OpenUI foundation verification — 20 October 2026

Historical Milestone 1 snapshot. Current implementation and verification: [GENUI.md](GENUI.md) and [implementation report](GENERATIVE_UI_IMPLEMENTATION.md).

Milestone 1 source and automated smoke verification is implemented. This is
not a declaration that the full generative UI mission or native acceptance is
complete. The existing uncommitted application work was preserved.

## Commands and results

| Command | Result |
| --- | --- |
| `npm run generate:openui` | Official CLI generated four prompt/spec pairs; full prompt copied to Rust |
| `npm run typecheck` | Passed |
| `npm test` | 701 passed, 1 skipped; 101 test files passed, 1 skipped |
| `npm run build` | Passed; existing large main/editor chunk warning remains |
| `cargo test --manifest-path src-tauri/Cargo.toml` | 762 passed, 17 ignored; binary/doc suites contained no tests |
| `cargo test --manifest-path src-tauri/Cargo.toml tool_calls::tests` | Final fence-guard source: 10 passed |
| `cargo test --manifest-path src-tauri/Cargo.toml chat::finish_tests` | Final fence-guard source: 41 passed |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | Failed: 89 diagnostics in existing code, including unused functions, needless borrows, argument counts and test assertions; no diagnostic targets the new guard |
| `git diff --check` | Passed |
| `npm run benchmark:openui` | Passed; offline CPU/bundle report below |
| Impeccable detector over changed UI targets | No findings |

The full Rust run executed successfully in this host environment; ignored tests
are not counted as passes. The final focused runs followed the last fence-scan
performance adjustment. Strict Clippy is still an unmet repository-wide check;
this milestone does not suppress warnings or rewrite unrelated dirty work.

Playwright/browser preview was **not run**, following AGENTS.md's explicit
preview prohibition. No alternate proxy, Playwright or browser route was used.
Native Tauri rendering, URL opening, actual model inference and app restart
were **not verified**. DOM Renderer tests do not establish those properties.

## Measured foundation overhead

Reproduce after building with `npm run benchmark:openui`. Raw measurements:
[`benchmarks/openui-foundation.json`](benchmarks/openui-foundation.json).
Host: Linux x64, Ryzen 5 7535HS, 12 logical CPUs, approximately 15.9 GB RAM;
Node 22.22.1. Thirty warmups then 300 samples per source fixture.

| Validation fixture | Source characters | p50 ms | p95 ms |
| --- | ---: | ---: | ---: |
| Core text | 73 | 0.096 | 0.143 |
| Supplied sales table | 80 | 0.086 | 0.140 |
| Literal Python code | 80 | 0.045 | 0.076 |
| Invalid progress | 41 | 0.048 | 0.077 |

Generated prompt sizes: full 5,967 characters / 6,019 UTF-8 bytes; core
4,809 / 4,851; data 5,355 / 5,403; coding 4,989 / 5,033. These are **not token
counts**. The existing opt-in setting still uses full; profiles are ready for
later policy integration.

Current production OpenUI shared runtime/catalog/validation chunk: 209,217
bytes (64,527 gzip). Chat wrapper: 2,714 (1,188 gzip); canvas wrapper: 671
(454 gzip). These are current chunk sizes, **not incremental size against a
controlled baseline**. The benchmark process peaked at 164,208 KiB RSS;
that includes Node/Vite/module-loading machinery and is **not desktop app RAM**.

Not measured: baseline Markdown generation, selected-model UI generation,
additional prompt tokens, time to first visible UI, inference throughput,
React/browser paint, interaction latency or native peak RAM. No small-model
reliability claim follows from parser/DOM fixture success.

## Source changes in this milestone

- `package.json`, `package-lock.json`: exact OpenUI/Zod pins, direct core
  dependency, development CLI, generation/test/benchmark commands.
- `src/genui/catalog.ts`, `library.tsx`, `genui.css`: bounded curated components,
  layout/typography primitives, shared highlighted code, themed focus/motion,
  responsive containers and safe Markdown content.
- `src/genui/specification.ts`, `scripts/build-genui-prompt.mjs`: React-free
  official library definitions, pruned profiles and CLI prompt/spec generation.
- `src/genui/generated/{full,core,data,coding}.{txt,spec.json}` and
  `src-tauri/src/genui_prompt.txt`: synchronized generated assets.
- `src/genui/validation.ts`, `ValidatedRenderer.tsx`, `VisualErrorBoundary.tsx`,
  `actions.ts`: preparse limits, explicit Zod validation, contained rendering
  and bounded action allowlist.
- `src/genui/GenUiSurface.tsx`, `src/components/GenUiBlock.tsx`: shared validation
  and actions for existing chat/canvas, source retained behind explicit toggle.
- `src/genui/{validation,actions,specification}.test.ts`,
  `src/genui/ValidatedRenderer.test.tsx`, `src/components/GenUiBlock.test.tsx`:
  real Renderer fixtures, adversarial input and generated-asset parity checks.
- `src-tauri/src/tool_calls.rs`, `src-tauri/src/chat.rs`: presentation fences
  excluded from text tool detection/parsing/stripping; executable calls outside
  fences preserved; Rust regressions cover both boundaries.
- `scripts/benchmark-openui.mjs`, `docs/benchmarks/openui-foundation.json`:
  reproducible offline foundation measurements.
- `docs/GENUI.md`, `docs/OPENUI_ARCHITECTURE_AUDIT.md`, this report: architecture,
  dependencies, component contract, generation, security and remaining work.

There is no database migration. Interactive state, original-answer preservation,
per-message format/library metadata and conversation overrides remain later
milestones. The source-only independent review resolved its three listed
findings; neither it nor documentation review establishes visual/native quality.
