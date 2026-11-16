# Conversation, sessions and harness: implementation and verification plan

This plan extends the existing work recorded in `HARNESS_WORKSPACE_2026-11-04.md` and `GENERATION_QUALITY_2026-11-04.md`. The working tree contains those implementations; their reported results are historical evidence, not a substitute for checks of later changes. Preserve the existing response-presentation decision and project instructions.

## Product outcomes and acceptance

1. **Conversation work:** Show actual streamed reasoning, commentary, tool actions, failures, approvals, plans and changed files in a readable timeline. Preserve order and scope; never invent a model's hidden reasoning or elapsed time. User disclosure choices should survive streaming and completion. Compare light/dark, narrow/wide, keyboard and long output in the native app when permitted.
2. **Independent sessions:** Start and switch among conversations while other conversations run. Own streams, drafts, interruption, approval and errors by conversation. Preserve current work when a tab closes. Test overlapping starts, completion, cancellation, delayed metadata and failed recovery, including app restart and real providers. Parallel runs in the same folder share files; isolated worktrees require a separately designed and tested workflow.
3. **Delegated work:** Expose actual parent/child task, status and output, input requests, cancellation, completion and errors; enforce scoped authority and concurrency limits. Add worktree isolation only with explicit lifecycle, cleanup, conflict and recovery semantics. Do not claim concurrent subagents from concurrent chats alone.
4. **Implementation quality:** For code, frontend, games, services and 3D, inspect the actual project and instructions, select available tools, define observable outcomes, implement end to end and verify with appropriate checks. Visual acceptance needs permitted rendered inspection; editable 3D delivery needs real geometry and a validated export. Guidance is not evidence that a model actually followed it.
5. **Efficiency and cache:** Measure time to first visible output, end-to-end turn duration, tool latency, token use and provider-reported cache hits against a fixed benchmark corpus. Preserve stable prompt prefixes and tool schemas, keep dynamic data out of cacheable prefixes, bound tool results and avoid redundant work. Test local slot behavior under real concurrency. Do not claim speedups or cache gains without comparable measurements.
6. **Provider correctness:** Validate reasoning/content streams, fragmented tool calls, complete tool-result replay, cancellation, errors, context limits and usage for OpenAI-compatible, Responses and Anthropic paths. Run actual account tests only with user-authorized providers and do not save credentials in source or fixtures.

## Research

- [River inference quickstart](https://docs.river.ai/guides/inference/) specifies its OpenAI-compatible endpoint, streamed events and reusable unchanged prefixes. [River API controls](https://docs.river.ai/guides/inference-controls/) specifies GLM-5.3-Flash's required reasoning and the separate `reasoning_content` and `content` fields. [River tools](https://docs.river.ai/guides/inference-tools/) requires assembling streamed tool arguments and returning results against the call ID.
- [Claude Code desktop](https://code.claude.com/docs/en/desktop) and [subagents](https://code.claude.com/docs/en/sub-agents) describe independent sessions and delegated work. [Codex worktrees](https://learn.chatgpt.com/docs/environments/git-worktrees) describes isolated parallel work. These are behavioral references, not claims that this project implements their private internals.

## Current pass

- The existing session implementation has conversation-scoped run controls, tabs, status indicators, drafts and a work panel. The corresponding mocked frontend and Rust tests do not prove native parallel inference, filesystem isolation or end-to-end provider behavior.
- The activity timeline now retains reasoning across multiple assistant messages, without duplicating journal entries, and leaves the disclosure under the reader's control throughout streaming and completion.
- Completion refreshes use a revision to reject outdated snapshots; failure to refresh conversation metadata no longer marks recovered response messages as failed, and a slow metadata refresh does not keep a finished turn busy. Regression coverage exercises overlapping refreshes and failed refreshes.
- A small authorized direct River GLM-5.3-Flash streaming request returned reasoning and answer chunks and a normal end marker. It did not exercise the Tauri app, model tool calls, provider setup UI, or visual layout.
- The focused App and ActivityTimeline suite passed (53 tests), as did TypeScript checking and the production build (with the existing chunk-size warning). A full frontend run recorded 858 passed, one skipped and one failure: the CanvasPanel OpenUI lazy-render test timed out waiting for `Total`. An isolated repeat also failed while the build was running; a later isolated repeat after the build passed all three CanvasPanel tests. No product or test timeout was changed. The full run is not reclassified as passing.

## Remaining work and validation

- Inspect and interact with real running desktop views and obtain visual acceptance; browser localhost preview is excluded by the project's instructions. No inference endpoint can substitute for inspecting the UI.
- Exercise concurrent real provider and local runs, task switching, restart, queued approvals, scoped stop, stale metadata and shared-folder conflict behavior. Validate model-specific tool loops and native history replay with real provider responses.
- Evaluate the authoring guides on representative frontend and 3D tasks with real outputs, applicable render/export tools and a scored benchmark. Add image generation only if an actual authorized image-generation provider or tool is available; GLM text inference is not an image-generation service.
- Design and implement any missing isolation, child-result inspection, durable recovery and cross-session ownership discovered by those tests. Benchmark prompt caching and tooling, then implement only measured improvements.
- Run the applicable frontend and Rust suites, typecheck and build after changes. Record failures, skipped tests and platform limits as observed.
