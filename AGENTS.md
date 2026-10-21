# LocalLM agent notes

Tauri app: Rust backend in `src-tauri/`, React/TypeScript frontend in `src/`.

## Verification commands

- Rust library suite: `cd src-tauri && cargo test --lib` (current checkpoint:
  968 passed, 0 failed, 17 ignored)
- Rust check: `cd src-tauri && cargo check --lib`
- Frontend tests: `npx vitest run` (770 passed, 1 skipped); focused:
  `npx vitest run src/components/ToolControls.test.tsx`
- Typecheck: `npm run typecheck` (`tsc --noEmit`)

## Environment limitations observed in sandboxed sessions

- TCP socket creation fails with `Operation not permitted`; real HTTP/socket
  fixtures (MCP HTTP transport, provider loopback, A11/A12 fixtures) must run
  on a permitted host. Do not report them as passing here.
- Browser localhost preview was declined by the user; do not bypass it with
  alternate browser/proxy/Playwright routes.

## Conventions

- Tool results use the versioned `locallm.tool-envelope/1` contract; normalize
  once at model-visible boundaries (`tool_envelope.rs`).
- Tool contracts live in `tool_spec.rs`: a `ToolSpec` carries the name, the
  description, the input schema and an optional output schema. Output
  contracts are enforced at the dispatch boundary; a contract that names an
  unknown tool fails the test suite (`tool_contract_regressions.rs`).
- The frontend's `invoke()` names are checked against the registered Tauri
  commands by a test. A mismatch is a runtime failure no Rust test could
  otherwise see.
- Large results are artifact-captured via `artifacts::bound_tool_result`;
  lifecycle handles (processId, proposalId, stagedTree) must survive bounding.
- MCP interactive input goes through `mcp_input.rs` (typed forms, per-call
  ownership, `resolve_mcp_input` command); detached calls carry no UI
  authority.
- Native provider context (Responses/Claude) persists via `history.rs`
  (`native_tool_rounds`, `model_message_contents`) anchored on tool audit rows;
  text-parsed calls and mixed program+model groups replay canonically.
- Workspace document tools (`workspace_documents.rs`) sniff format from bytes,
  use no-follow scoped opens, and return image data only as artifact-captured
  model image parts.
- Conversation rendering uses shared primitives — `MarkdownView`, `CodeBlock`,
  `DiffView`, `lib/highlight.ts` (highlight.js subset + aliases + LRU cache),
  `lib/ansi.ts` (stateful ANSI parser). Semantic colors live only in
  `tokens.css` (`--cat-*` activity categories, `--syn-*` syntax, `--code-*`,
  `--diff-*`, `--terminal-*`); do not scatter color literals in components.
  Contract and palette notes: `docs/RENDERING.md`.
- The working tree intentionally contains uncommitted work; do not reset,
  stash, or clean it.

## Removed homepage control — do not restore

- The user explicitly removed the response-presentation selector on 21 October
  2026 (the `Default (auto)` / Auto / Text / Visual dropdown). Do not implement
  this selector, an equivalent response-mode control, or per-conversation mode
  editing in the homepage or chat composer unless the user explicitly requests
  it again. This is a product decision, not missing functionality.
- Response presentation is configured in Settings only. The chat's presentation
  hook is read-only for context preflight; do not add mode overrides to submission.
- Older audits, plans, benchmark logs and screenshots describing the removed
  selector are historical evidence, not instructions to restore it. This rule
  supersedes their composer-mode requirements.
