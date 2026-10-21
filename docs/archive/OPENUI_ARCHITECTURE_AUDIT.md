# OpenUI architecture audit — 20 October 2026

Historical Milestone 1 snapshot. Current implementation and verification: [GENUI.md](GENUI.md) and [implementation report](GENERATIVE_UI_IMPLEMENTATION.md).

Scope: local working tree, including the pre-existing uncommitted GenUI work.
Milestone 1 extends that implementation rather than installing a second UI.

## Existing lifecycle

- `App.tsx` creates conversations, saves tool selections, adds an optimistic
  user row, and calls `api.sendMessage`. Its channel callback assembles content
  and reasoning deltas by message ID and tracks tool streams, approvals and run
  state. Persisted messages are reloaded after the run.
- `api.ts` invokes `send_message` with `Channel<ChatEvent>`. `chat.rs` assembles
  a `TurnPlan`, selects the existing inference backend, budgets context, runs
  planning/tool/finalization rounds, and emits deltas. It checkpoints streaming
  messages roughly every 500 ms and persists terminal status.
- Events contain `messageId`, `content`, `reasoning`, optional run/step/sequence,
  state/activity, context, approval, round timing and tool-stream events. There
  is currently no presentation-format event or final-answer-only stream.
- `Chat.tsx` still has `MessageBody`, but no longer owns ReactMarkdown:
  `MarkdownView` is the shared pipeline. Assistant OpenUI fences lazy-load
  `GenUiBlock`; `GenUiSurface` renders the same library in the canvas.
- `store.rs` owns SQLite conversations/messages, runs/events, artifacts,
  research sessions/tasks and settings. Messages have content, separate
  reasoning, status and error; native provider replay lives in separate
  `model_message_contents` / `native_tool_rounds` records. Visual source is
  currently just fenced message text. There is no interactive-state table.

## Prompts, context and security

- `prompt.rs` builds one frozen leading system message, stable tool schemas,
  history, then the current user draft with volatile injections. The existing
  `generativeUi` preference adds `genui_prompt.txt` to the frozen system.
  `chat.rs::context_preflight` uses the same assembly and token-counting
  backend; existing UI prompt bytes already count as instruction overhead.
- Local llama.cpp, Responses, Claude and other remote adapters remain behind
  the existing inference interface. `history.rs` preserves provider-native
  replay anchored to tool audit rows. Presentation must not alter those rows.
- Tool parsing/execution, AREX finish, research finalization and claim-audit
  footers are in the agent path, separate from React rendering. Permissions
  remain enforced by the harness, approval and permission modules. External
  tool results are data, not authority for UI actions.
- Tauri registers commands in `lib.rs`. CSP permits self assets and IPC,
  forbids frames/objects and does not allow eval. Capabilities include opener
  and dialog access, so React action handlers are a real trust boundary.
  At audit time, GenUI URL handlers lacked scheme validation and unknown
  actions fell through to follow-ups. The foundation resolves this with the
  shared allowlist in `actions.ts` and URL schema in `catalog.ts`.

## Foundation changes and compatibility risks

Keep `src/genui` as the single catalog/library location. Pin the already
installed React Lang/core/Zod versions, bound schemas and program resources,
generate prompt plus official library spec together, reuse shared code
rendering, and smoke-test actual Renderer output and partial streams. Reject
invalid programs safely in both chat and canvas. No Node server, CSP change,
database migration or replacement harness is needed for this milestone.

Subsequent milestones must add an explicit final-answer boundary, Auto/Text/
Visual policy, capability/context-based library selection and additive
message-owned presentation/state persistence. Merely parsing arbitrary model
generations as UI would break tool planning and research finalization. Adding
new mode state to the existing frozen prefix each turn would harm caching;
mode-specific instructions need deliberate suffix placement and matching
preflight accounting. Stored programs need a library version and an original
factual answer before reliable Markdown fallback or upgrades can be promised.

Bounded validation intentionally rejects previously accepted oversized or
invalid programs; preserve their source for inspection/export. Existing
Markdown storage and interpretation stay intact. Renderer smoke tests are
not evidence of native inference, SQLite state restoration or model reliability.
