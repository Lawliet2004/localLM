# Context-window checks and Execution Harness

New installations default to MiniCPM5-2B's 131,072-token context. Existing saved configurations are preserved. Saving Runtime settings while a model is loaded reloads it so the new context size is actually allocated. If the model is not loaded, Models & runtime still shows saved versus loaded context when they differ, with **Apply saved configuration** to reload. A reload allocates the requested KV cache; a larger context uses more memory. The context is shared by input, tool results, and the response reserve. Re-using the same model from the library keeps the saved context; it no longer resets to the first-load 8,192-token default. The response reserve cannot consume the entire window: at 8,192 context it is capped at 4,096 tokens so a typical first prompt still fits.

The model-visible prompt is built for llama.cpp prefix caching (`cache_prompt`). A frozen system block (user system prompt, skill guidance, response style, and CODE-preset PTC SDK) and a frozen per-conversation tool catalog sit first. Conversation history is replayed next using original tool-call ids and grouped parallel calls. Volatile injections (memory, plan/todos, unavailable-connector notices) are a trailing system block immediately before the current user message, so they do not invalidate the cached prefix. Token counting uses `/v1/chat/completions/input_tokens` without `cache_prompt` or `id_slot` so composer preflight cannot evict the generation slot.

Before a new message is saved, LocalLM asks the loaded llama.cpp runtime to count the complete generation payload. It includes the chat template, the frozen system block, selected tool definitions, conversation history, the trailing scratchpad, and the draft. The app reserves the fitted maximum response tokens. If prompt plus response exceeds the loaded context, it rejects the send and restores the draft with a token-count explanation.

After tool results arrive, the next model round is counted again. If a result makes the context too large, the turn ends with a saved error; already completed tool actions and their audit records remain available. No history is silently removed or summarized.

## Draft preflight counting

While you type, a debounced count runs against the same rendered payload a send would build — the turn assembly is shared code, so the breakdown cannot drift from what is sent. The composer shows exact (or provider-estimated) totals split into instructions (frozen system prompt, skills, static style), tool schemas, replayed history, trailing scratchpad, and the draft, plus the response reserve and loaded context. When the draft plus reserve exceeds the context, an overflow notice explains the options before anything is sent. Counting uses the same authenticated `/v1/chat/completions/input_tokens` endpoint; failures hide the indicator instead of guessing.

## Compaction

Compaction is a bounded checkpoint plus recent complete turns, never a deletion. `compact_conversation` (model tool, Composer "Compact history" button, or `compact_conversation_cmd`) copies the dropped prefix verbatim into an artifact readable back through `artifact_read`, records a cutoff in the `compaction` table, and replays only the kept suffix with a checkpoint notice. Boundaries fall only on user-message starts, so a tool audit is never separated from the turn that requested it. Full transcripts, audits, and session events stay in SQLite. Automatic compaction is enabled by default and can be disabled per conversation (`set_compaction_auto`). Before each generation round, LocalLM checks input plus the response reserve; at 80% of the loaded context capacity it archives older completed exchanges, persists a user-boundary cutoff so the next turn does not resurrect dropped history, re-counts, emits a visible compaction event, and continues the same run. Trailing volatile injections stay after the frozen system and checkpoint notice. If the current message plus tool catalog alone exceeds capacity, the count after compaction still reports the overflow instead of trimming further.

## Tool profiles

The Tools page offers explicit Standard, Chat, Research, and Coding profiles. Chat injects a minimal harness (no web, shell, or subagents); Research exposes selected live search/fetch connectors with repository documentation tools opt-in; Coding focuses on workspace editing and execution. Profiles are per-conversation config patches over tool visibility: switching conversations preserves each conversation's saved profile and custom connector selections, and a profile chosen for a new chat is persisted before its first turn.

## Tool output Bounding & Artifact Store

Tool outputs inserted into the conversation context are strictly bounded by `artifacts::bound_tool_result()`. The excerpt budget follows the tokens still free after the counted input and the response reserve (at most half of the remainder, clamped to 512–8,192 characters); any output beyond it is trimmed into a structured preview snippet, while the unabridged payload is persisted with its sha256 checksum in the SQLite `artifacts` table. The message history retains `_artifactId` and byte sizing metadata, allowing the model (`artifact_read`) and the user (Inspect artifact) to reach the full result without blowing the context budget.

## Authoritative Run Harness & Sequence Invariants

Every conversation generation round is managed through `agent_run::RunRecord` with monotonic event sequences (`seq`), explicit step identifiers (`step-0`..`step-8`), and validated state transitions (`Preparing` -> `Generating` -> `AwaitingApproval` -> `ExecutingTools` -> `PreparingNextRound` -> `Completed` | `Failed` | `Cancelled`).
Live activity events are streamed to the frontend via Tauri IPC channels, rendering real-time activity dots and status badges in the chat interface.

## Truncation & Finish-Reason Protection

When an inference stream terminates with `finish_reason == "length"` and the round produced tool calls, LocalLM treats this as unrecoverable: partial tool arguments are rejected, the run fails, and the assistant row is saved with `status: "error"`. A text-only length finish with auto-compaction enabled appends a user continuation instruction (never inserted into the frozen prefix) and continues the same run. If continuation is off, the same error as above is saved.

The first tool-catalog snapshot for a conversation is stored in `conversation_prompt_freeze`. Later turns reuse those schemas even if a connector is briefly down (the call then returns an unavailable error). Changing Tools, skills, or the preset replaces the freeze and is an accepted cache miss. Auto-compaction persists a user-boundary cutoff so the next turn replays the compacted prefix instead of resurrecting dropped history.

## Deterministic Local System Time Tool

To eliminate model hallucinations regarding time and date without polluting system prompts, LocalLM provides a built-in deterministic `system_time` tool. It resolves IANA timezones (such as `Asia/Kolkata` -> `+05:30`) and ISO offsets, outputting ISO 8601 strings, UNIX epochs, daylight saving indicators, and explicit `"local_system_clock"` source attribution.
