# Harness and workspace upgrade — 4 November 2026

## Purpose and direction

LocalLM should support coding, research, documents, and everyday assistance equally. The working surface should make it easy to leave a running conversation, work elsewhere, inspect a plan or delegated work, and return to a readable answer. Preserve the existing paper/forest theme, typography, shared Markdown/code/diff renderers, access controls, stored history, and Settings-only response presentation.

## Research and source evidence

These are design and architecture references, not dependencies or copied implementations. Revisions were read from the public repositories on 4 November 2026.

| Reference | Evidence read | Application to LocalLM |
| --- | --- | --- |
| [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) | `8bd073392eef6f54e921a093b95076248eb08cbb`; [session package](https://github.com/deepseek-ai/deepseek-harness/blob/8bd073392eef6f54e921a093b95076248eb08cbb/packages/core/session/README.md), `src/index.ts`, `src/fork.ts`; [3 November release](https://github.com/deepseek-ai/deepseek-harness/releases) | Event-derived sessions, explicit ownership, independent forks, reliable restoration. Use LocalLM's existing SQLite session journal rather than introducing another session format. DeepSeek describes this harness as a developer preview. |
| [Codex platform documentation](https://developers.openai.com/blog/codex-as-a-platform) and [public harness](https://github.com/openai/codex) | `d84c872464aca4821dc5cb548a651651aefd84fa`; `codex-rs/app-server/README.md`, `core/src/agent/control/interrupt.rs`, `spawn.rs` | Thread/turn identity, streamed activity, scoped interruption, separate approvals and agent lifecycle. Adapt these patterns to the existing Tauri protocol. This is not an audit of the proprietary desktop app source. |
| [Claude Code desktop](https://code.claude.com/docs/en/desktop) and [sub-agents](https://code.claude.com/docs/en/sub-agents) | Official docs describe independent parallel sessions, per-session projects, agent tools/permissions, foreground/background execution. Public repository `26382c08758260c9f5deb2019d8fe851f89fa9ae`: `README.md`, `mods/agents-md/README.md`. | Independent work contexts and visible delegated work. Public extension code does not establish access to the private desktop implementation; no such source-access claim is made. |
| [OpenCode](https://opencode.ai/changelog) (supplementary initial research) | `55276f2108c8853f94cf16dc530e7754a6ce8318`; `packages/app/src/session/composer/subagent-bar.tsx`, `todos.ts`; [agents documentation](https://opencode.ai/docs/agents/) | Concise actual-state labels and separation of plan progress from model prose. No SolidJS components or competing design system are imported. |

The user clarified the primary references as Codex, Claude Code, and DeepSeek Harness. “Dexter” was not established as a particular product and is excluded from factual comparisons. Command Code desktop documentation was consulted during disambiguation only.

## Findings in LocalLM

- The Rust operation gate permits independent conversations, but App.tsx holds one mutable response, error, pending request, and approval dialog, and disables navigation during generation.
- Cancel/pause controls remain global; enabling parallel UI without scoping these would allow one conversation to interrupt another.
- Sub-agent lineage, individual interrupt, goals, todo state, session forks, and activity events already exist in the backend; much of this is absent or difficult to find in the UI.
- Response rendering already has canonical Markdown, math, code, diff, source, terminal, and artifact components. Improvements should extend these rather than create another renderer.

## Implementation plan

1. **Conversation ownership:** isolate in-flight message buffers, notices, errors, context, approval requests and cancellation; keep one turn per conversation, retain the shared/exclusive runtime gate, and keep global configuration changes guarded while any turn runs.
2. **Session navigation:** add persistent open-session tabs, keyboard switching, visible working/attention/completed states, and independent drafts. Closing a tab only changes navigation. Existing sidebar search, projects and history remain available.
3. **Work visibility:** expose the real plan, objective and sub-agent lineage in a compact, optional conversation work panel. Show honest empty/loading/error states, inspect each agent's task, and stop an individual running agent through the existing backend command. Surface plan-before-action through the command palette and work panel, including its approval semantics.
4. **Answer quality:** add navigation across long answers and a usable complete-answer copy action; improve responsive typography and the surrounding session/work chrome without overriding saved appearance preferences or changing generated content.
5. **Verification and handoff:** test independent streams, approvals and stop ownership, switching while running, stale async responses, tab keyboard behavior, and agent state/error handling; run typecheck, production build, frontend tests and the Rust library suite/check. Record actual results and limitations below.

## Boundaries

- No new provider, credential, subscription, private protocol or model-weight fine-tuning. “Fine-tune” here means improve the application's harness and experience.
- Parallel sessions can share a project directory. They are not automatically isolated worktrees; existing worktree tools remain available. Local inference throughput depends on configured slots and hardware.
- No new response-presentation selector in the composer or homepage.
- Browser localhost preview is excluded by AGENTS.md. No proxy, alternate browser or Playwright route will bypass that decision. This pass can establish code and contract quality; visual/native acceptance must be reported as unverified unless actual permitted evidence is available.

## Verification results

Implemented:

- Open conversations have saved tabs, arrow/Home/End keyboard navigation, close-tab actions, working/input-needed/new-response indicators, and independent drafts. Closing a tab leaves running work intact. The sidebar still provides access to closed running sessions.
- In-flight messages, tool output, notices, errors, context and approval queues stay with their originating conversation. Completion in a hidden conversation does not replace the visible answer. Rejected unsaved prompts restore their original draft rather than the currently visible draft.
- Tauri sends establish an owned live-control scope; nested foreground tools inherit its cancel/pause signal. Stop is addressed by conversation. Old clients may omit the ID only when the target is unambiguous. Exit cancels all live scopes. Conversation-local tools/model/preset/project mutations retain the global operation gate and also acquire their own conversation's turn lock.
- Workspace navigation writes are serialized and selection-version checked. A delayed earlier selection cannot replace the current project's identity. Independent model turns resolve their workspace from their own stored task association.
- Single local parent requests retain slot 0 for cache reuse. Competing parent sessions let llama.cpp choose an available slot. Actual simultaneous inference and throughput depend on configured slots and hardware; no speedup was measured. The upstream [server scheduling implementation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/server-context.cpp) was consulted for its handling of explicitly requested busy slots.
- **Work** opens a responsive conversation panel with the stored objective, checklist, **Plan before acting**, and owned sub-agent records. Agent task details show run/parent lineage and failures; running agents have individual Stop controls. Planning is also discoverable through Ctrl+Shift+P. It uses the existing read-only investigation and plan-approval gate.
- Completed prose answers with at least three rendered headings gain **In this answer** navigation. It reads the canonical renderer's headings, excluding exposed reasoning, and introduces no second Markdown parser. **Copy answer** is visible. Existing math, code, sources, diffs and artifacts keep their existing renderers and policies.

Completed checks:

| Check | Result |
| --- | --- |
| Frontend full suite | 855 passed, 1 skipped (120 files). |
| Focused final UI regression set | 84 passed: App, Chat, session tabs and conversation work. Includes concurrent streams, scoped Stop, hidden draft restoration, queued approvals, reversed navigation completion, stale agent responses, individual agent Stop and rendered-heading navigation. |
| Rust full library suite | Final run with `--test-threads=4`: 982 passed, 0 failed, 19 ignored, 64.36 s. `cargo check --lib` also passed. |
| Timing-sensitive native regression | An initial default-concurrency full run passed (982/0/19). A later run concurrent with the full frontend suite reported 981 passed, 1 failed, 19 ignored: `chat::read_group_tests::real_turn_cancel_settles_every_dispatched_member` exceeded its 900 ms wall-clock bound. An isolated diagnostic did not reproduce the failure (0.60 s). The final constrained full run above is a separate observation; the failed run is not reclassified as passing. No threshold was relaxed. |
| Typecheck / production build | Both passed after the final ownership corrections. Build reports existing large JavaScript chunks. |
| Impeccable detector | `[]` over all changed UI targets. Ran once. |
| Whitespace check | `git diff --check` passed. |

The final 84-test focused set ran after the approval-order and stale-callback corrections; the full frontend suite also includes the deliberately delayed first-workspace-write regression. No product timeout or test threshold was relaxed. The independent finish reviewer confirmed the navigation ownership fix resolved at CODE-ONLY scope and requested the completed evidence wording above.

Final finish-review disposition: **ship at CODE-ONLY scope**. The reviewer scored the navigation ownership and documentation-accuracy fixes resolved. This is approval of those scored fixes, not rendered visual or native interaction acceptance. The independent documenter found an ordinary extension of the incumbent system and preserved its system files.

No browser preview, screenshot comparison, native interaction acceptance, real-provider inference benchmark or model training was performed. Ignored native/socket fixtures remain unverified and are not counted as passing. The build's chunk-size warning and JSDOM's existing canvas diagnostic remain visible; this work adds no runtime dependency. Theme contrast, narrow layout, zoom and visual finish have code-level safeguards but no new rendered acceptance evidence.


## Design documentation review

Compared `PRODUCT.md` and `DESIGN.md` with `src/tokens.css`, `src/sessions.css`, the session tabs, conversation work panel, answer navigation, Chat, Sidebar, ToolControls, `App.tsx`, and `useSessionRuns.ts`. This is an ordinary extension of the incumbent system: paper/forest surfaces and semantic state colors, IBM Plex Sans controls, existing mono task identifiers, restrained borders, shared renderers, labelled controls and focus treatments. Session navigation, optional work inspection and answer navigation add behaviors without establishing a new palette, type scale or visual world. No durable design-system change was found; `PRODUCT.md` and `DESIGN.md` remain unchanged.

Source review checked keyboard tab navigation, task and lineage disclosure, individual Stop controls, plan-before-action copy, canonical rendered-heading navigation, visible answer copying, reduced-motion handling and narrow-window rules. This establishes code-level intent only. Browser preview remains prohibited, and this review performed no screenshot, native/browser interaction, rendered contrast, zoom or visual acceptance check. The previously recorded detector result was not rerun.

Pre-existing documentation drift was not canonized or repaired: `DESIGN.md` retains its historical format and evidence, and `.impeccable/design.json` is absent in this checkout. Updating the system format or creating a sidecar is outside this extension. Earlier visual evidence does not establish acceptance of these new surfaces.
