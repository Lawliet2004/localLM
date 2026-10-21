# Backend Audit — 21 October 2026

**Scope.** `src-tauri/src/` — the entire Rust backend of the Tauri desktop app.
93,729 lines of Rust across 129 files, 188 registered Tauri commands, ~125
model-facing tools, 4 provider backends, 7 presets, 836 passing Rust tests and
114 frontend test files.

**Mode.** Read-only audit. No application file was modified. Every finding
below was re-verified against the source by the auditor before inclusion; the
P0 was additionally reproduced empirically against the verbatim schema and
triggers from `store.rs`.

**Comparison baseline.** OpenCode V2 (`packages/core` at `anomalyco/opencode`
`dev`), Claude Code v2.1.x (`code.claude.com/docs`), and Codex CLI. OpenCode
source was read at source level; Claude Code and OpenCode documentation was
read directly.

> **Concurrent-work note.** A `codex --yolo` process was editing this
> repository throughout the audit. `skills.rs`, `lib.rs`, `presentation.rs`,
> `tool_spec.rs` and `src/genui/*` all changed mid-session, and the crate was
> observed in a **non-compiling state twice** (a duplicate `list_skills` in
> `skills.rs`, then `tool_spec.rs` test-target errors). Line numbers below are
> from the working tree as of the audit; the defects are re-verifiable by
> symbol name. A snapshot was taken to `/tmp/opencode/audit-snap`.
>
> **Re-verified against the latest tree after the final edit pass:** B1 is
> still live — the recovery `UPDATE` is still inside `Store::initialize`'s
> `execute_batch` (`store.rs:442`), no `Store::recover_interrupted` exists,
> and all thirteen live-DB second-connection sites are unchanged
> (`chat.rs:1882`, `chat.rs:1919`, `subagents.rs:109/1052/1649/1707`,
> `lsp_tools.rs:570/587/841`, `development_tools.rs:28/32`,
> `verification_observer.rs:112`).

---

## 0. Verdict

**The backend is well-hardened at the edges and weak in the middle.**

Every *external* boundary is genuinely good — path handling, shell invocation,
SSRF on the researched path, the credential vault, MCP protocol bounds, the
approval fan-out, and the tool-result envelope are all correct and mostly
tested. The earlier 16 October findings A01 (workspace symlink escape) and
A03 (DNS rebinding) are **fixed** and confirmed fixed.

The failures are **internal**: state that is destroyed mid-turn, context that
silently disappears, budgets that do not apply to the code paths that need
them, and a build that is not gated. One finding is a genuine P0 — silent
permanent loss of provider context on every tool call that touches the
database.

| Severity | Count | Headline |
|---|---|---|
| **P0** | 1 | Second DB connection re-runs crash recovery → native provider context destroyed mid-turn |
| **P1** | 9 | Compaction drops all prior history; a finalizing round can kill the turn; children bypass turn budgets; model-spawned processes survive app exit; downloads are never reclaimed |
| **P2** | 22 | Contract gaps, SSRF inconsistency, `panic = "abort"`, no CI gate, unstructured errors, 34 ad-hoc HTTP clients, god-function, unbounded artifact table |
| **P3** | 15 | Confused-deputy scope ids, budget accounting, dead code, cache eviction, orphaned LSP sessions |

**Total: 47 findings, all verified. All 47 are now fixed** — see
[Remediation status](#part-x--remediation-status) for the file-by-file record
and the verification results.

---

## PART I — P0

### B1 — Every second connection to the live database re-runs startup crash recovery, destroying native provider context mid-turn

**Severity: P0** · silent, permanent, data-destroying.

**Where.** `src-tauri/src/store.rs:442-443` (inside `Store::initialize`'s
`execute_batch`), `src-tauri/src/store.rs:298-301` (trigger
`native_tool_round_edit`), `src-tauri/src/store.rs:312-316` (trigger
`invalidate_model_history_on_message_update`),
`src-tauri/src/presentation_store.rs:22`.

```rust
// store.rs:442-443 — inside the schema batch, so it runs on EVERY Store::open
UPDATE messages SET status='interrupted' WHERE status='streaming';
UPDATE runs SET status='cancelled' WHERE status IN
  ('preparing','generating','awaiting_approval','executing_tools','preparing_next_round');
```

`Store::open` → `Store::initialize` (`store.rs:209-211`). These are
**startup crash-recovery** statements, but they live in the *schema* function,
so every additional `Store::open` on the live database re-runs them.

**The trigger chain.**

1. `chat.rs:3665` / `chat.rs:3668` — the tool audit row is created with
   `status = "streaming"`.
2. `chat.rs:3671` → `history::remember_native_round` (`history.rs:47`) →
   `store.save_native_tool_round(&row.id, …)` — the opaque native provider
   context (OpenAI Responses items / Claude content blocks) is anchored on
   that row in `native_tool_rounds`.
3. The tool is dispatched. If it routes through `catalog_dispatch`
   (`chat.rs:1882`) or `run_catalog_and_record` (`chat.rs:1919`), a **second
   `Store::open(live_db)`** is issued → the recovery batch fires → the audit
   row flips `'streaming' → 'interrupted'`.
4. The tool settles → `chat.rs:4044` `update_message(&row.id, …, "complete")`
   → trigger `native_tool_round_edit` fires with `OLD.status='interrupted'`:

```sql
CREATE TRIGGER IF NOT EXISTS native_tool_round_edit AFTER UPDATE OF content ON messages
WHEN OLD.content!=NEW.content AND OLD.status!='streaming' BEGIN
    DELETE FROM native_tool_rounds WHERE message_id=NEW.id;
END;
```

The `OLD.status!='streaming'` clause exists **precisely** to protect in-flight
audits. The recovery `UPDATE` defeats it.

**Empirical reproduction** (verbatim schema and triggers from `store.rs`,
rusqlite/SQLite semantics):

```
BASELINE                                 status=streaming    native_tool_rounds=1  conv_model_history=1
AFTER 2nd Store::open(live db) recovery  status=interrupted  native_tool_rounds=1  conv_model_history=0
AFTER foreground settle (status=complete) status=complete     native_tool_rounds=0  conv_model_history=0
```

**Actual.** The native provider context is deleted mid-turn. `history.rs:148-152`
then finds no `contents` entry, so `flush_tools` (`history.rs:266-282`) falls
back to the **canonical** replay — a synthetic assistant message with aliased
tool names (`local_run_code_<hash>`), no provider item ids, and **no reasoning
items**. Provider reasoning continuity is lost. Separately,
`invalidate_model_history_on_message_update` fires on the recovery `UPDATE`
itself, destroying `conversation_model_history` (the prompt-cache prefix
checkpoint) — so **every** second connection open busts the prompt cache for
the whole conversation.

**Reachability — how many second connections exist.**

| Site | When |
|---|---|
| `chat.rs:1882`, `chat.rs:1919` | every `run_code` / `file_read` / `file_write` / catalog tool call |
| `lsp_tools.rs:570`, `:587`, `:841` | every LSP tool call |
| `subagents.rs:109`, `:1052`, `:1649`, `:1707` | every subagent child |
| `development_tools.rs:28`, `:32` | evidence save/load |
| `verification_observer.rs:112` | after every verification observation |

**The broadest trigger is `subagents.rs:1707`:**
`Store::open(&snapshot.db_path)` where `snapshot.db_path = state.db_path`
(`subagents.rs:479`) — a **second connection to the live database held for the
entire background-subagent lifetime**. Spawning one background child therefore
flips **every** `'streaming'` row in **every** conversation to `'interrupted'`.

**Preconditions.** The native-round path requires the Responses or Claude
backend (`history.rs:47` is only reached for provider-native rounds). The
audit-row flip requires the tool to open a second live connection. Both are
routine, not exotic.

**Expected.** Crash recovery runs **once**, at startup, on the single
authoritative connection.

**Fix.**

1. Move `store.rs:442-443` out of `Store::initialize` into a one-shot
   `Store::recover_interrupted(&self)` (mirroring the existing
   `research_tasks::recover_interrupted` at `research_tasks.rs:389`) and call
   it exactly once from `lib.rs:263-265`, where `research_tasks::recover_interrupted`
   is already invoked.
2. Do the same for `presentation_store.rs:22`.
3. Add a non-recovering constructor (`Store::open_shared(path)`) for the live-DB
   call sites, or route all of them through the existing `AppState::database()`
   mutex so there is exactly one connection to the live database.
4. Regression test: native round saved → `Store::open_shared(live)` → settle the
   audit row → assert `native_tool_rounds` still holds the row.

---

## PART II — P1

### A1 — Auto-compaction silently deletes the entire prior conversation from the model-visible prompt

**Where.** `src-tauri/src/compaction.rs:142-247` (`compact_model_messages`),
called from `src-tauri/src/chat.rs:2484` inside `prepare_context`
(`chat.rs:2452-2505`).

```rust
// compaction.rs:147-156
let Some(user) = messages.iter().rposition(|m| m["role"] == "user") else { return Ok(None) };
let tail = messages
    .iter().enumerate()
    .skip(user + 1)              // looks only AFTER the last user message
    .rfind(|(_, m)| m["role"] == "assistant")
    .map(|(i, _)| i)
    .unwrap_or(messages.len());   // no assistant after the draft ⇒ tail == len
// compaction.rs:174-185
if i < leading || volatile == Some(i) || research_checkpoint == Some(i)
   || i == user || i >= tail { kept.push(...) } else { dropped.push(...) }
```

**Why it always fires.** `build_turn_messages` (`prompt.rs:128-138`) builds
`[system(frozen)] + plan.history + [draft]`. The **draft is the last message**,
so `skip(user + 1)` is an empty range → `tail = messages.len()` → the
`i >= tail` branch is unreachable → the keep-set reduces to
`{system, draft}`.

**Actual.** Every prior user message, assistant answer, tool result and native
round is dropped. Only 8 × 180-char excerpts survive (`compaction.rs:198-217`),
plus a "use `artifact_read`" hint. Reasoning items and Claude thinking blocks
live on the dropped assistant messages and are lost with them.

**Expected.** The function's own doc comment says *"Keep instructions, the
latest user request, and the latest complete exchange"* (`compaction.rs:139-141`).
`docs/CONTEXT.md:17` says compaction is *"a bounded checkpoint plus **recent
complete turns**"*. `compaction_keep_last` (default 10 turns,
`compaction.rs:299-310`) exists to define that window — but
`compact_model_messages` **never reads it**.

**Impact.** Any conversation crossing 80 % of the context (routine on an 8k
local model with a 2.5k system prompt plus tool schemas) gives the model the
current question with **no memory** of the previous question, the previous
answer, or any tool result. The model restates, contradicts, or re-runs work.

**Fix.** Compute the retention window over the **history region**, not after
the draft: find the last `keep_last(store)` user-message boundaries *before*
`user` (a boundary may only fall on a user message so a tool pair is never
split) and keep everything from the earliest of those onward. Keep the
existing `tail` rule for the current turn's exchanges.

---

### A2 — A finalizing round containing a tool call corrupts the next provider request

**Where.** `src-tauri/src/chat.rs:3231-3233` and `chat.rs:3281-3282`;
consumed by `src-tauri/src/responses_adapter.rs:111-115` and
`src-tauri/src/claude_adapter.rs:158-165`.

```rust
// chat.rs:3231-3233
if !calls.is_empty() {
    suppressed_call = true;
    let finished = std::mem::take(&mut calls).finish().unwrap_or_default();  // calls is now EMPTY
    if let Some(call) = finished.iter().find(|call| call.name == "finish") { /* … */ }
}   // ← no finish call ⇒ the tool call is silently discarded
// chat.rs:3281-3282 (FinalizationAction::Retry)
let native_reply = protocol.assistant_message(&round_answer, &[]);   // canonical calls = []
if calls.is_empty() && crate::history::saved_native_message(&native_reply).is_some() {
    messages.push(native_reply);                                     // native call set ≠ canonical set
}
```

**Trigger.** A *finalizing* round in which the model emits a tool call and no
usable text. `finalizing` is routinely reachable: research budget exhausted
(`chat.rs:4089-4092`), `Continuation::Partial` (`chat.rs:3991-3996`),
`budgets.reserve_final_response()` (`continuation.rs:102-111`),
`finalization_retried` (`chat.rs:3274`). For local AREX the finalizing payload
deliberately keeps `tool_choice: auto` (`chat.rs:4573`), so a local AREX model
**can and does** emit non-`finish` calls on a finalizing round. Providers that
ignore `tool_choice: "none"` do the same.

**Actual — Responses backend (unconditional failure).**
`protocol.assistant_message` (`sse.rs:203-221`) attaches the round's *native*
output: `ResponsesNative::replay()` (`responses_adapter.rs:617-623`) returns the
provider's **unfiltered** output items. The canonical `tool_calls` array is
built from `&[]`. The next request therefore fails at
`responses_adapter.rs:111-115`:

```
Err("Saved Responses function calls conflict with canonical history.")
```

This propagates through `plan.backend.payload(…)` at `chat.rs:2861` via `?`,
so **the whole turn dies** with an internal-sounding message — after
`answer.truncate(answer_len_before_round)` (`chat.rs:3278`) has already
discarded the model's text.

**Actual — Claude backend (conditional).**
`ClaudeNative::replay(&[])` (`claude_adapter.rs:84-107`) returns `None` unless
the round produced a `thinking` / `redacted_thinking` block. When it does
return, the captured `tool_use` block is **not removed** (the `calls` loop only
adds), so the assistant message carries a `tool_use` with no matching
`tool_result` — which the Anthropic Messages API rejects.

**OpenAI-chat backend is unaffected** — `Backend::payload` strips
`locallm_responses_output` and `locallm_claude_content` for non-native
backends (`inference.rs:461-476`).

**Secondary defect on the same line.** `calls.finish().unwrap_or_default()`
(`chat.rs:3233`) swallows a malformed-call error (duplicate ids, invalid JSON
arguments — `tool_calls.rs:108-149`). The non-finalizing path correctly uses
`calls.finish()?` (`chat.rs:3347`). A malformed call on a finalizing round is
silently downgraded to "no calls", which also discards the model's text.

**Fix.** Mirror what the `finish` branch already does at `chat.rs:3249-3255`:
do **not** `mem::take` the calls. Keep them, build the assistant message with
`protocol.assistant_message(&round_answer, &finished)`, and push one explicit
tool result per call (a `suppressed-tool-call` notice with `SIDE_UNKNOWN` for
the non-`finish` ones) so the canonical and native call sets stay identical.
Replace `unwrap_or_default()` with `?` or an explicit `FinalizationAction::Retry`.

---

### A3 — Subagent children bypass the turn's search/fetch budget and the turn's replay-protection set

**Where.** `src-tauri/src/continuation.rs:9-28` (`admit_tool`),
`src-tauri/src/chat.rs:2762-2778` (registration),
`src-tauri/src/subagents.rs:1420-1450` (`call_child_tool`).

```rust
// continuation.rs:9-28
pub(crate) fn admit_tool(state: &AppState, run_id: &str, name: &str) -> Result<(), String> {
    let cell = state.turn_budgets.lock()…?.get(run_id).cloned();
    if let Some(cell) = cell { …budget.try_note_tool(name)… }
    Ok(())          // ← unknown run id ⇒ no budget at all
}
```

The budget cell is registered **only** under the parent run id
(`chat.rs:2763`). `call_child_tool` sets `run_id: child_run_id`
(`subagents.rs:1428`) and calls `crate::harness::execute` directly
(`subagents.rs:1449`) — **it never calls `admit_tool`**. Even if it did, the
child run id has no registered cell and `admit_tool` would silently pass
(`continuation.rs:17-26`).

The **PTC** nested path does it correctly — it uses the parent's `run_id` and
charges the shared cell before dispatch (`ptc.rs:858`, `ptc.rs:1129`). The
chat loop's own dispatch also charges it (`chat.rs:2169`, `chat.rs:3661`).
**Only the subagent path is missing.**

**Actual.** `Budgets::max_searches = 16` / `max_fetches = 16`
(`continuation.rs:83-96`) do not bound a child: a child runs up to 8 rounds
(`subagents.rs:1140`) and each `search` call may carry up to 4 queries
(`arex.rs:170-173`) ⇒ up to 32 provider searches per child, with **zero** charge
to the parent — and the parent may spawn several children.

`state.turn_blocked` — the "uncertain outcome ⇒ never repeat" set the parent
consults before every dispatch (`chat.rs:2776-2778`, `chat.rs:3606-3611`) —
is **not** consulted by children either. A side-effecting call the parent
refused to repeat because its outcome was uncertain can be re-executed verbatim
by a child.

**This contradicts the documented contract** at `chat.rs:2759-2761` (*"nested
dispatch paths (plan steps, parallel reads) charge searches/fetches against the
same counters before they run, so loops cannot overspend the turn"*) and
`continuation.rs:113-117`.

**Fix.** Give the child the parent's budget cell: pass the parent `run_id` (or
the `Arc<Mutex<Budgets>>`) into `ChildRequest` / `HarnessCtx`, and call
`crate::continuation::admit_tool(state, &parent_run_id, alias)` in
`call_child_tool` before `harness::execute`, mirroring `ptc.rs:858`. Likewise
look up `state.turn_blocked` for the parent run before dispatching a child call.

---

### A4 — The global connectors mutex is held across network handshakes

**Where.** `src-tauri/src/connectors.rs:165-170` (startup restore) and
`connectors.rs:1140-1156` (manual connect); spawned at `src-tauri/src/lib.rs:268-272`.

```rust
// connectors.rs:165-170
pub async fn restore_connections(state: &crate::AppState) -> Result<(), String> {
    let mut hub = state.connectors.lock().await;          // global tokio Mutex<McpHub>
    let ids = remembered_connections(&*state.database()?)?;
    hub.restore_saved(&ids).await;                         // ← network, under the lock
    Ok(())
}
```

`restore_saved` (`connectors.rs:777-786`) calls `connect()` per saved connector
**sequentially**. `connect()` (`connectors.rs:886-1072`) performs an MCP
handshake (30 s timeout, `:965`/`:982`), tool discovery (30 s, `:998-1011`) and
a 60 s reqwest timeout (`:913`).

**Actual.** `assemble_turn` (`chat.rs:1061`) awaits `state.connectors.lock()` →
the **first chat turn after launch blocks** until every connector restore
finishes. With *N* saved connectors on a flaky network that is up to
*N* × ~60 s of a hung first message. `list_connectors` (`connectors.rs:1099`)
and `skills.rs:919` block too. `connect_connector` has the same shape.

**Fix.** Take the hub lock only to snapshot ids and to insert the finished
connection; perform the handshake off-lock, then `lock()` to commit. Or give
`McpHub` an internal per-connector map so the outer mutex is never held across
I/O.

---

### A5 — The global skills mutex is held across a network download

**Where.** `src-tauri/src/skills.rs:846-852` → `Skills::install`
(`skills.rs:465-543`), one HTTP GET per file with a 45 s timeout each
(`skills.rs:493-521`).

**Contenders.** `chat.rs:1141` (`assemble_turn` → `skills.instructions`),
`skills.rs:659`/`:685` (`skill_list` / `skill_load` harness tools),
`skills.rs:845` (`list_skills`), `:900` (`read_skill_file`), `:908`
(`skill_update_status`), `:921` (`skill_dependencies`).

**Actual.** One skill install freezes chat-turn assembly and every skill read
for the whole download.

**Fix.** The codebase already knows the pattern: `skills.rs:658` and `:684` do
`let manager = ctx.state.skills.lock().await.clone();` — clone the manager out
of the lock and install without holding it, or move to a per-skill lock.

---

### A6 — `SubagentRegistry` panics on a poisoned mutex instead of returning an error

**Where.** `src-tauri/src/subagents.rs:664, 667, 677, 683, 695, 707, 715, 722,
731, 735, 739` — eleven `.expect("subagent registry")` calls on a
`std::sync::Mutex`.

Every other lock in the backend degrades gracefully: `AppState::database`
(`lib.rs:170-174`) → `Err("Database lock is unavailable. Restart the
application.")`; `mcp_input::lock` (`mcp_input.rs:58-60`) →
`unwrap_or_else(|e| e.into_inner())`; `terminal_ui::lock`
(`terminal_ui.rs:160-164`) → `Err`; `daytona_cleanup::journal` → `Err`.

**Actual.** One panic while holding `flags` / `cancels` / `mailbox` poisons that
mutex, and every later `register` / `interrupt` / `retire` / `deliver` panics —
a permanent, self-inflicted denial of the subagent feature.
`SubagentRegistry` is a process-wide `OnceLock` singleton
(`subagents.rs:753-758`), so it is never rebuilt.

**Fix.** Replace all eleven with
`self.flags.lock().unwrap_or_else(|e| e.into_inner())`, matching
`mcp_input.rs:58-60`.

---

### A7 — A synchronous `git log` subprocess inside an async Tauri command

**Where.** `src-tauri/src/memory.rs:114-138` → `survey_repo` →
`src-tauri/src/memory.rs:68-71`.

```rust
let commits = std::process::Command::new("git")
    .args(["-C", &path.to_string_lossy(), "log", "--oneline", "-10"])
    .output()          // blocking: waits for the child to exit
```

`ingest_repo` is `pub async fn` → it runs on a tokio worker thread and blocks
it for the whole subprocess lifetime (seconds on a large repository).

**Fix.** `tokio::process::Command`, or `tokio::task::spawn_blocking`.

---

### A8 — Every model-spawned child process survives app exit

**Where.** `src-tauri/src/lib.rs:494-508` (the only exit hook),
`src-tauri/src/execution.rs:344-357` (`managed_spawn`),
`src-tauri/src/process_tools.rs:405-417`, `src-tauri/src/sandbox.rs:157-283`.

**The exit hook** (`lib.rs:494-508`) cancels the run, kills `pty_sessions`,
stops automation and stops the runtime. It does **not** touch
`state.processes` (`ProcessRegistry`), `state.terminals`
(`TerminalRegistry`), `state.debuggers`, `state.language_servers` or
`state.connectors`.

**Why the registries cannot clean up after themselves.** `tauri::App::run` is
documented as *"This function never returns. When the application finishes, the
process is exited directly using `std::process::exit`"*
(`tauri-2.11.5/src/app.rs:1346`, `:1366`). It delegates to
`tao::EventLoop::run`, which calls `process::exit(exit_code)` —
verified at `tao-0.35.3/src/platform_impl/linux/event_loop.rs:998` (and
`:230` on Windows, `:202` on macOS). **No destructors run.** `AppState` is
never dropped, so:

* `ProcessRegistry` is never dropped → the `mpsc::Sender<Control>` inside each
  `Process` (`process_tools.rs:198`, `:405-414`) is never dropped → the
  supervisor's `receiver.is_closed()` check (`process_tools.rs:682-691`) never
  fires → the supervised child is never `terminate()`d;
* `TerminalRegistry` is never dropped → each `TerminalSession.child`
  (`sandbox.rs:148`, `:201-212`) never gets `ExecutionChild::drop` → no
  `SIGKILL` to the group;
* `debug_tools`, `lsp_client` and `connectors` are likewise never dropped.

**Compounding this**, `managed_spawn` sets `ProcessGroup::leader()` →
`setpgid(0,0)`, so children are in a **new process group** and do **not**
receive the controlling terminal's `SIGHUP`; and `managed_spawn` does **not**
set `PR_SET_PDEATHSIG`.

That last point is a **known, deliberate asymmetry in this codebase**.
`runtime.rs:305-334` (`runtime_command`) *does* set PDEATHSIG, with an explicit
comment: *"Dev restarts and crashes can bypass Drop and Tauri's Exit handler.
Release GPU memory even when the app is forcibly terminated."* The mitigation
was never applied to `managed_spawn` — the path used by `process_tools`,
`sandbox::TerminalRegistry`, `debug_tools`, `lsp_client`, `local_mcp_process`,
`profile_tools` and `workspace_ui::git`.

**Actual.** A `process_start` / `shell_run` child (`python -m http.server`,
`npm run dev`, `sleep 3000`) or a persistent `terminal_create` shell is
orphaned and keeps running, reparented to init. The registry entry, the
supervisor task and the output buffer die with the app; the OS process does
not. The `process_start` contract — *"conversation-owned process ID … use
`process_stop` to terminate the process tree"* (`process_tools.rs`) — is
unenforceable after exit: the ID is gone, the process is not.

**Fix.**
1. **Primary (deterministic):** extend `managed_spawn` with the same
   `PR_SET_PDEATHSIG` guard used at `runtime.rs:314-325` (Linux only; harmless
   elsewhere). This makes the OS kill the child when the parent dies, covering
   crashes, `SIGKILL` and dev restarts.
2. **Secondary (graceful):** in the `RunEvent::Exit` handler, explicitly tear
   down the missed registries — add `ProcessRegistry::stop_all()`,
   `TerminalRegistry::close_all()`, `debug_tools::Manager::stop_all()`, and
   call `state.language_servers.shutdown().await` plus
   `McpHub::disconnect_all()`. `RunEvent::Exit` fires *before*
   `process::exit`, so that window is real and usable.

---

### A9 — Interrupted model downloads are never reclaimed (unbounded multi-GB disk growth)

**Where.** `src-tauri/src/download.rs:125-132` (`partial_path`),
`src-tauri/src/install_recovery.rs:5-11`, `:49-61`,
`src-tauri/src/lib.rs:273-279`, `src-tauri/src/model_install.rs:144`,
`src-tauri/src/model_library.rs:373`, `:401`, `:408`, `:651`.

The downloader writes partials under a `<name>.part` convention:

```rust
// download.rs:125-132
pub fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    destination.with_file_name(name)
}
```

The **only** startup garbage collector for interrupted installs recognises a
*different* naming scheme:

```rust
// install_recovery.rs:5-11, 49-61
fn owned(name: &str, prefix: &str, suffix: &str) -> bool {
    name.strip_prefix(prefix).and_then(|n| n.strip_suffix(suffix))
        .is_some_and(|r| r.len() == 32 && r.bytes().all(|b| b.is_ascii_alphanumeric()))
}
if owned(&file, ".locallm-download-", ".part") && regular(&model, &file) { model.remove_file(&file)?; }
```

**Verified:** `grep -rn "locallm-download" src-tauri/src/ scripts/` returns
6 hits — 4 inside `install_recovery.rs` itself and 2 in smoke scripts.
**Nothing in the Rust downloader ever creates a `.locallm-download-*` file.**

| Producer | Real partial path |
|---|---|
| `model_install::install_model` (`model_install.rs:144`) | `<app_data>/models/<catalog-filename>.part` |
| `model_library::download_hugging_face_model` (`model_library.rs:373`, `:401`, `:408`, `:651`) | `<app_data>/models/<64-hex-digest>/<basename>.part` |

Neither matches `owned(name, ".locallm-download-", ".part")`, so
`install_recovery::clean` **never removes a single real partial**.

The only path that removes them is `model_library::cleanup_model_storage`
(`model_library.rs:825-831`), which runs **only** when the user explicitly
deletes that model.

**The smoke test passes vacuously.** `scripts/model-cancel-smoke.mjs:26`
asserts that `readdirSync(...).filter(name => name.startsWith('.locallm-download-'))`
is empty — true, because the downloader never produces that prefix. The real
`<name>.part` left behind is not asserted.

**Actual.** Cancel or fail a multi-GB model download and the `.part` file
survives. Repeat across models and each leaves a full-size partial. There is
**no quota**, **no age-based sweep**, and **no working startup collector**, so
`<app_data>/models/` grows without bound — until the app's own "Not enough disk
space" preflight (`download.rs:276-279`) starts failing for unrelated
downloads.

Note that `download::fetch` deliberately **preserves** partials for resume
(`download.rs:415-427`: `discard` is true only when `saved == 0 ||
saved > asset.bytes || error.contains("SHA-256") || error.contains("truncated")`).
That is correct for resume; the missing piece is the collector.

**Fix.** Either (a) make the producer match the consumer — derive the partial
name from the digest so it is `.locallm-download-<32 hex>.part` and keep
`partial_path` consistent — or (b) extend `install_recovery::clean` to also
remove `*.part` files directly inside `<models>/<64-hex>/` and `<models>/*.part`,
guarded by the same `regular()` (non-symlink) check. Option (b) is ~10 lines
and also repairs already-corrupted installs. Additionally add a per-model-dir
partial budget so a single model cannot consume unbounded disk.

---

## PART III — P2

### Contract and protocol

**C1 — `tool_invoke` is advertised to the model but hard-fails inside every PTC plan.**
`harness.rs:578` returns `"tool_invoke must resolve to its original authorized
tool before dispatch; this execution path does not support discovery
invocation."`. The PTC plan path (`ptc.rs:695-718`) looks the alias up in
`ctx.inherit_tools` and dispatches it there; its guard excludes only
`ptc_run | finish | update_context` (`ptc.rs:706-708`). `chat.rs:1613`
explicitly instructs the model: *"Use tool_search to find names and exact input
schemas, then **tool_invoke** to call a discovered tool."* The Code preset has
70 tools (`presets.rs`), so `tool_discovery::attach` is always active there and
`tool_invoke` is always advertised. `tool_search` *does* work inside a plan
(`harness.rs:574-577`) — the model is told to search-then-invoke and only half
of that pairing is implemented. The identical call works in the parent chat
loop (`chat.rs:3500-3513`) and in the subagent driver (`subagents.rs:1217-1232`).
*Fix:* resolve the wrapper in the PTC path exactly as the parent loop does, and
implement the `tool_invoke` arm in `harness::execute_inner`.

**C2 — `shell_run` loses its `processId` and its "running" state when bounded.**
`artifacts.rs:350-366` (`lifecycle_excerpt`) covers only `check_run` /
`check_report`, the debug tools, and `process_start|poll|write|stop`.
`shell_run` is absent, so it falls to the generic path whose whitelist
(`artifacts.rs:712-731`) does not contain `processId`.
`process_tools.rs:576-580` returns `{"status":"running","processId":id,…}` on
timeout. After bounding, the model sees `status:"success"` (the *envelope*
status — `merge_compat` refuses to overwrite the envelope's own `status` key,
`tool_envelope.rs:148-154`) and the real `"status":"running"` plus `processId`
survive only inside a JSON-escaped preview string, truncated to ~80–200 chars
at the 512-byte floor. The tool's own advertised contract — *"On timeout the
process keeps running and the returned processId continues the lifecycle through
process_poll and process_stop"* (`process_tools.rs:40`) — is broken: a
still-running child becomes unreachable.
*Fix:* add `shell_run` to the lifecycle set and read the `processId` key for it.

**C3 — The `finish` tool result reaches the model raw, without the envelope.**
`chat.rs:702-706` builds `json!({"answer":…,"evidences":…,"confidence":…})` and
returns it at `chat.rs:769` with no `normalize()` and no bounding. The same
tool through the normal loop is normalized (`chat.rs:3932`) into
`{schemaVersion, status, data:{answer,evidences,confidence}, …}`. On the
finalizing path a **raw** result and an **envelope** result
(`suppressed-tool-call`) sit side by side in one transcript
(`chat.rs:3250-3254`), the durable audit row is a third shape
(`chat.rs:708-712`), and the frontend renders the two differently
(`src/components/Chat.tsx:86-90`). This violates the contract stated at
`tool_envelope.rs:3-15` (*"Every tool result that reaches the model uses one
envelope"*).
*Fix:* `tool_envelope::normalize(json!({…}), Family::Harness)` at this boundary,
and use the normalized value for the audit row, the control application, the
event and the transcript push.

**C4 — PTC `parallel` blocks skip the dispatch-time capability-kill re-check.**
The sequential path checks it (`ptc.rs:710-717`); `run_parallel`
(`ptc.rs:1126-1143`) performs budget admission, schema validation, replay
protection and approval — but **not** `capabilities::tool_capability(tool)`.
Harness members are accidentally safe (`harness::execute` re-checks inside
`execute_inner`, `harness.rs:486-492`); MCP members are accidentally safe
(`concurrent_read()` excludes them, `connectors.rs:426-431`); **native**
members are not. This violates the invariant stated at `harness.rs:484-485`.
*Reachability is honest:* `set_capability_enabled` takes the global
`state.operation` lock (`capabilities.rs:478-481`), which the chat loop holds
for the whole turn, so the current UI cannot flip a kill mid-turn. This is a
defense-in-depth gap, not a live bypass.
*Fix:* factor the check into a helper and call it in `run_parallel`.

**C5 — One budget unit can perform up to four provider requests.**
`try_note_tool` (`continuation.rs:118-141`) charges **one** unit per tool
*name*, but `arex::strings` accepts 1–4 queries (`arex.rs:170-173`) and the
harness `search` / `visit` handlers forward the whole batch in one call
(`harness.rs:525-530`, `:552-553`). The effective ceiling is ~4× the configured
`max_searches` / `max_fetches`.
*Fix:* charge per element.

**C6 — Anthropic's `refusal` and `pause_turn` stop reasons are surfaced as
"unsupported finish reason".** `claude_adapter.rs:386` maps unknown stop
reasons with `other => other`; `check_finish_reason` (`chat.rs:4248-4255`)
accepts only `"stop" | "tool_calls" | "length" | "content_filter"`. A
legitimate Anthropic `refusal` ends the turn with *"Model returned an
unsupported finish reason: 'refusal'"* and the model's refusal text is
discarded. `pause_turn` (server-tool pause) and
`model_context_window_exceeded` are affected the same way.
*Fix:* map `refusal` to a normal non-tool completion and `pause_turn` to a
resumable state.

**C7 — Repeated `id` / `name` in streaming tool-call deltas are concatenated
instead of replaced.** `tool_calls.rs:91-101` `push_str`s `id` and `name`
alongside `arguments`. OpenAI's contract is that `id` and `function.name`
appear in the **first** delta only; several OpenAI-compatible servers
(llama.cpp-based proxies, some gateways) repeat them in **every** delta. The
result is `id = "call_abc"+"call_abc"+…` and `name = "read"+"read"+…`, which
then fails to resolve (`chat.rs:3500-3502`) and is reported to the model as
`unavailable-tool` — typically provoking a retry loop.
*Fix:* replace rather than append for `id` and `name`; keep append semantics
for `arguments`.

### Security

**C8 — `docker_exec`: a model-controlled container image is spliced unvalidated
into a `docker run` command line.** `harness.rs:978-981` prefers the model's
`image` value over the **user-pinned** `sandbox::docker_image(&store)`.
`harness.rs:209` declares the schema as `"image": {"type":"string"}` — no
pattern, no `maxLength`, no allow-list. `sandbox.rs:60-65` places it in the
**image slot** of the argv with no `--` separator. Docker's CLI stops flag
parsing at the first positional argument, so a value beginning with `-` is
parsed as a **`docker run` flag**: `--privileged`, `--net=host`,
`--pid=host`, `--cap-add=SYS_ADMIN`, `--volume=<host-path>:<container-path>`,
or `--volume=//./pipe/docker_engine://./pipe/docker_engine` (Docker socket →
full host takeover). The user-pinned image is bypassed entirely, so an
attacker-chosen registry image runs with the workspace mounted **read-write**.
*Scope:* `docker_exec` is in the **Creator** preset only (94 tools), and
`C:\work` is not an absolute container path on a Linux daemon, so this is
latent today — but the argument-validation bug is unambiguous.
*Fix:* remove the model-supplied `image`, or validate it against a strict
image-reference regex (rejecting a leading `-`, whitespace and control
characters) and pass it after a `--` separator.

**C9 — The SSRF guard is applied to `web_fetch` only.** `sandbox.rs:383-483`
already provides `is_blocked_ip` and `vet_fetch_target`, and
`sandbox.rs:622-650` already provides a pinned, proxy-disabled, per-hop
re-vetted transport — plus the Node research worker has an independent
equivalent (`src-tauri/resources/web/worker.mjs:6161-6210`). Those are **not
reused** by:
- `api_tools.rs:60-71` (`url()`) — scheme/host/credential check only, **no
  private-address filter**;
- `api_tools.rs:155-163` (`client()`) — default `reqwest::Client`, **no
  `.no_proxy()`, no DNS pinning**;
- `api_tools.rs:174-186` (`http_session_create`) — `base_url` accepted unchecked;
- `api_tools.rs:241-268` / `:365` — the response body is returned to the model;
- `openapi_tools.rs:604-655` — the base URL may come from a **workspace
  OpenAPI document the model can write**;
- `automation.rs:333-348` (`validate_browser_call`) — any http/https host,
  including `http://127.0.0.1/` and `http://169.254.169.254/`.

A model can therefore read cloud-metadata IAM credentials
(`http://169.254.169.254/latest/meta-data/iam/security-credentials/<role>`)
or probe internal services through `http_request`.
*Honest scoping:* these tools are not trusted reads and are not `always-ask`,
so they need explicit approval in Ask / Auto-approve modes; they run
unattended only in Full access, where the model can already `curl` via
`execute_command`. This is a **policy inconsistency and defense-in-depth gap**,
not a privilege escalation.
*Fix:* make `sandbox::vet_fetch_target` / `is_blocked_ip` public and reuse them;
build those clients with `.no_proxy()` + `resolve_to_addrs` pinning; re-vet each
redirect hop.

**C10 — The Google Programmable Search key is the one first-class credential
stored in cleartext and returned to the renderer.** `web_search.rs:387` stores it
in the plain `settings` table (`store.rs:287`); `web_search.rs:354` and `:360`
return it verbatim as `"googleApiKey"`. `grep -c vault src-tauri/src/web_search.rs`
= **0**. Every other credential uses the AES-256-GCM vault: provider keys
(`providers.rs:222`), connector tokens (`connectors.rs:800`), OAuth
(`oauth.rs:30-37`), database passwords (`remote_database.rs:85`), HTTP dev
credentials (`development_credentials.rs:56`), the Daytona key
(`daytona_settings.rs:138`), local MCP configs (`local_mcp_config.rs:142`).
Provider read APIs return only a `has_api_key` boolean (`commands.rs:59-66`).
*Adjacent, same class:* `execution.rs`'s `bash.environment` / `python.environment`
and `lsp_client::ServerConfig::environment` are plain JSON in `settings`, so
shell/LSP env vars carrying tokens are cleartext — undocumented.
*Fix:* move the Google key into the vault, keep a presence flag in settings,
return `hasGoogleApiKey`.

### Process and architecture

**C11 — No CI runs the backend, and the documented release check is red.**
`.github/workflows/` contains exactly one workflow (`development-tools.yml`),
scoped to the development-tools feature. It runs **no** `cargo test`, **no**
`cargo clippy`, **no** `npm test`, **no** `npm run typecheck`. The README
documents `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D
warnings` as a release check — that command **currently fails with hard
errors** in the lib-test target. Across 457 modified/untracked files, the crate
was observed **non-compiling twice** during the audit, and nothing would have
caught either.
*Fix:* add a `backend` job: `cargo fmt --check`, `cargo clippy --all-targets
-- -D warnings`, `cargo test --lib`, `cargo check --all-targets`; a `frontend`
job: `npm run typecheck`, `npm test`; and run both on every push to `src-tauri/**`
and `src/**`.

**C12 — `panic = "abort"` in the release profile, with a Windows-only crash log.**
`src-tauri/Cargo.toml:62` sets `panic = "abort"`. Any backend panic kills the
whole app — every open conversation, unsaved draft, in-flight tool subprocess.
The panic hook (`lib.rs:182-200`) persists to
`%LOCALAPPDATA%\com.locallm.app\panic.log`, **Windows only**; on Linux/macOS
the panic text goes to stderr, which a GUI bundle discards. Two of three
platforms get **no crash diagnostics at all**. `tool_contract_regressions.rs:27`
and `:221` use `catch_unwind` to assert panic-recovery semantics that cannot
exist in the shipped binary.
*Fix:* write the crash log to a platform-correct location (XDG_STATE_HOME /
`~/Library/Logs` / `%LOCALAPPDATA%`) with a size cap and rotation; consider
`panic = "unwind"` so a panic in one tool task does not take down the app.

**C13 — Every error is a `String`.** 179 `Result<_, String>` sites; **zero**
error enums, zero error codes, zero source chains, zero `Error` impls. The
English message *is* the API contract, shared simultaneously by the frontend,
the model, and the audit trail. This blocks programmatic UI error handling,
retry classification, structured telemetry, and machine-readable model-facing
errors. OpenCode V2 gets all four from Effect's tagged errors.
*Fix:* introduce a small `Error` enum per domain with a stable `code` and
`kind`; keep `Display` for humans and add a serialized `{code, message, kind}`
for machines.

**C14 — 34 independent `reqwest::Client` construction sites.** Each with its
own timeout, redirect, proxy, cookie and retry policy. `sandbox.rs:627-650`
has an excellent pinned-DNS transport; `api_tools.rs`, `openapi_tools.rs`,
`connectors.rs`, `download.rs`, `providers.rs`, `mcp_http.rs`, `inference.rs`,
`daytona.rs`, `runtime.rs`, `process_tools.rs` and `context.rs` each build
their own. SSRF/timeout/redirect policy is enforced only where it was
remembered — which is exactly how C9 arose.
*Fix:* one `http_client` module with named constructors
(`public_pinned()`, `internal_pinned()`, `provider(base, key)`, `download()`)
that encode the vetted policy once.

**C15 — No schema migration framework.** `store.rs:250-443` uses
`CREATE TABLE IF NOT EXISTS` + ad-hoc `pragma_table_info` probes +
`ALTER TABLE … ADD COLUMN`; `presentation_store.rs`, `research_tasks.rs` and
`daytona_journal.rs` each do the same. `PRAGMA user_version` is **never** read
or written anywhere in the backend. There is no way to detect a database
written by a newer build, to order the additive steps, or to know which have
run. A failed `ALTER` (locked DB, disk full) is indistinguishable from
"column already exists" in the surrounding flow.
*Fix:* set `PRAGMA user_version = N` after each successful migration and gate
the additive steps on the stored value.

**C16 — `send_message_inner` is 1,711 lines.** `chat.rs` is 6,811 lines with
137 functions; the entire turn orchestration — assemble → context prep → stream
→ tool dispatch → approval → compaction → persist — is one function. Not
isolatable in tests; every edit risks the whole turn.
*Fix:* extract the round loop (`chat.rs:2906+`) into a `Turn` struct with
`prepare / stream_round / dispatch_calls / finalize` methods, and the test
module into `chat/tests.rs`.

**C17 — One global mutex serializes the entire application.**
`send_message_inner` (`chat.rs:2548-2551`) takes `state.operation` and holds
it for the **whole turn** — assembly, network, every model round-trip, all
tool execution. A second conversation's send fails with *"Another model
operation is in progress."* `cancel_generation` correctly bypasses the lock
(`chat.rs:1752-1757`), so cancellation works. OpenCode V2 uses a per-key
`SessionRunCoordinator` (`packages/core/src/session/run-coordinator.ts`) with a
documented "doorbell" protocol that closes the lost-wakeup gap between a
drain's last eligibility check and the idle transition; localLM has neither
per-session isolation nor wake-up correctness.
*Fix:* replace the global mutex with a per-conversation coordinator; keep a
global one only for genuinely exclusive operations (model install, credential
change).

**C18 — Blocking Secret Service / D-Bus calls on the tokio runtime.**
`vault.rs:57-94` calls `keyring::Entry::get_secret()` synchronously.
`McpHub::list()` (`connectors.rs:802-824`) performs one vault load per
connector plus the local-config loads, and is reachable from async Tauri
commands (`connectors.rs:1182`). A locked or unavailable keyring blocks a
tokio worker for the D-Bus timeout.
*Fix:* wrap vault I/O in `tokio::task::spawn_blocking`.

**C19 — Non-atomic multi-statement writes.**
`save_todos` (`store.rs:875-892`) issues a `DELETE` then *n* autocommitted
`INSERT`s — a failure on insert *n* leaves a partially replaced checklist.
`fork_session_events` (`store.rs:1420-1470+`) creates the forked conversation
then copies tools, prompt-freeze, every session event and every message as
separate autocommitted statements — a failure part-way leaves a half-forked
conversation. The codebase already has the right pattern
(`unchecked_transaction()` at `store.rs:1803`, `:2004`, `:2052`, `:2253`).
*Fix:* wrap both in one `unchecked_transaction()`.

### Resource lifecycle

**C20 — Deleting a conversation orphans its running processes, shells and
debugger sessions.** `commands.rs:291-301` (`delete_conversation`) stops only
the **automation** (browser) session:

```rust
state.automation.stop_conversation(&id).await;   // browser only
let store = state.database()?;
store.delete_conversation(&id)?;
```

The three other owner-scoped registries have no owner-scoped cleanup API:

| Registry | Owner key | Cleanup API |
|---|---|---|
| `process_tools::ProcessRegistry` | `owner` = conversation id (`process_tools.rs:194`, `:256-268`) | **none** — only `owned(owner, workspace, id)` lookup |
| `sandbox::TerminalRegistry` | `owner` (`sandbox.rs:152`, `:225-231`) | **none** — only `close(id, owner)` |
| `debug_tools::Manager` | `owner` (`debug_tools.rs:439`) | **none** — only `debugger_stop` per id |

**Actual.** The DB rows and their artifacts are removed (`ON DELETE CASCADE`,
`store.rs:315`) but the OS processes keep running with no reachable handle —
`process_stop` / `terminal_close` need an id, and the conversation that
produced it no longer exists, so no future turn can address it. The registry
entry also lingers until the 64-entry cap evicts it.
*Fix:* add `remove_owned(owner)` to `ProcessRegistry` (send `Control::Stop` for
each match and await the replies), `TerminalRegistry` and
`debug_tools::Manager`; call them from `delete_conversation`.

**C21 — The artifacts table grows without bound (no quota, no TTL, no
pruning).** `artifacts.rs:682-696` stores the **full, untruncated** raw result
(`content: raw.clone()`). `store.rs:347-358` has no retention column and no
expiry, and `grep -n "DELETE FROM" store.rs` shows **no** `DELETE FROM
artifacts` anywhere — the only removal path is `ON DELETE CASCADE` when the
*conversation* is deleted. The `size_bytes` column records the size but is
never used for enforcement, and no disk quota is enforced for the DB or its
WAL.
**Actual.** `locallm.sqlite` grows monotonically for the lifetime of a
conversation that repeatedly calls a tool whose output exceeds `max_chars`.
*Fix:* add a retention sweep (`DELETE FROM artifacts WHERE conversation_id=?
AND created_at < ?`) or a per-conversation byte budget in `save_artifact` that
evicts the oldest rows past a cap; and enforce `max_chars` **before**
`raw.clone()` so the DB never holds more than the declared budget.

**C22 — `Manager::shutdown()` can orphan a live LSP session (process +
semaphore permit) that becomes unreachable.** `lsp_client.rs:601-624`:

```rust
for slot in slots.values() {
    if let Ok(mut session) = slot.try_lock() {   // <-- fails if a request is in flight
        if let Some(session) = session.take() { retired.push(session); }
    }
}
slots.clear();                                   // <-- map entry dropped anyway
```

If `shutdown()` / `clear()` runs while a request is in flight:
1. `try_lock()` fails → the session is neither taken nor shut down.
2. `slots.clear()` drops the map entry; the `Arc<Slot>` survives only because
   the in-flight operation holds a clone.
3. The `tokio::select!` at `lsp_client.rs:804` has two ready branches
   (`revision.changed()` and a completing `operation`), and `tokio::select!`
   picks a **uniformly random** ready branch. If `operation` wins, line 804
   restores `Some(session)` into the now-orphaned slot.
4. The slot is no longer reachable from `self.slots`, so the 120-second idle
   reaper (`lsp_client.rs:628-648`) never sees it, `Session::shutdown` is never
   called, the LSP server process keeps running with no handle, and the
   `OwnedSemaphorePermit` (`lsp_client.rs:176`, from `Semaphore::new(4)`) is
   never returned.

**Actual.** After four such orphans, every LSP request fails with *"All 4
language server sessions are busy"* (`lsp_client.rs:664-666`) — a permanent
functional outage for the LSP tools. Trigger: editing the LSP configuration in
Settings (`lsp_tools.rs:974-984` calls `state.language_servers.shutdown()`)
while an LSP request is in flight; also reachable from `semantic_impact.rs:895`,
`:1036`.
*Fix:* after `slots.clear()`, retain the removed slots and, for each whose
`try_lock()` failed, await the lock and `take()` + `Session::shutdown()` the
session. Alternatively give `Session` an `Arc<AtomicBool>` "retired" flag that
`shutdown()` sets, and have `request_guarded` drop rather than restore a
retired session.

---

## PART IV — P3

| # | Finding | Location |
|---|---|---|
| **D1** | **MCP input scope ids are per-session counters, not globally unique** — connector A's *k*-th call and connector B's *k*-th call both get scope `k`; `remove_scope` filters the **global** registry by that bare token, so one connector's cleanup can cancel another connector's pending form and wake its waiter with a close event labelled with the wrong connector. Reachable because `McpHub::connect` installs a **new** `SessionInput` (tokens restart at 0) and then cancels the old session. | `mcp_input.rs:242, 252, 272`; `:188-199`; `:290-313`; `connectors.rs:1055-1057` |
| **D2** | **The bounded model-visible envelope can exceed the excerpt budget by the full envelope overhead** — `minimal_envelope` shrinks the *inner* object to `budget`, then `rewrap_envelope` adds ~400 bytes of envelope overhead that is never re-checked. The existing regression passes only because its fixture is a **non-envelope** payload. | `artifacts.rs:1071-1089`; `:220-270` |
| **D3** | **No `PRAGMA user_version`** — see C15. | `store.rs:250-443` |
| **D4** | **`web_search_cache` eviction is a full flush, not LRU** — one heavy run wipes every other conversation's cached evidence; the key includes `run_id` so entries are never reused across turns. Bounded, so not a leak. | `harness.rs:539-544`, `:696-699` |
| **D5** | **Redundant DDL on a hot path** — `research_tasks::ensure_table` re-runs `CREATE TABLE IF NOT EXISTS` on every research-task access, but the schema batch already created it. `execute_batch` is DDL and takes the SQLite write lock; `save_research_task` is on the tool-dispatch path. | `store.rs:2419, 2445, 2479, 2496, 2520` → `research_tasks.rs:410-431` |
| **D6** | **`terminal_open` holds the `pty_sessions` std mutex across PTY creation** — the guard is alive across a PTY allocation, a process spawn and two thread spawns, blocking every other terminal command. The file already shows awareness of this class of problem at `terminal_ui.rs:393-394`. | `terminal_ui.rs:289-295` |
| **D7** | **`permission_rules.rs` is dead code** — 985 lines (including tests) that are **never compiled**: there is no `mod permission_rules;` in `lib.rs`, so its commands are not registered and `decide` / `decide_for_tool` are never called. The richer allow/deny/ask rule engine is therefore unreachable, so a user cannot say "never run `rm`". *Before wiring it:* `decide_for_tool` gives an `Allow` rule priority over the access mode, i.e. a rule can **widen** access; and the "a rule may never be created or changed by model output" invariant is currently enforced only by the command not being exposed. | `permission_rules.rs` (whole file); `lib.rs:1-113` |
| **D8** | **`opener:allow-open-path` + `dialog:default` are granted to the main webview with no path restriction** — anything that compromises the renderer can hand any file to the OS default handler. *Partly clean:* the main webview is the only capability entry, `lib.rs:480-489` rejects IPC from any other label, `core:webview:default` grants no `allow-create-webview`, and the inspector browser's URL parser rejects `file:` / `javascript:` / `data:` / credentialed URLs. | `src-tauri/capabilities/default.json:9-14` |
| **D9** | **A live run is reported as `cancelled` while the turn is still running** — a consequence of B1: `commands.rs:513-542` surfaces the recovery-flipped `runs.status` to the UI until the foreground's next `update_run_status`. | `store.rs:443`; `commands.rs:513-542` |
| **D10** | **`download::fetch` renames over an existing destination** — the `exists()` check and the `rename()` are not atomic, and Unix `rename(2)` silently replaces the destination, contradicting the error text *"an existing destination is preserved"*. | `download.rs:429-438` |
| **D11** | **The debugger registry keeps empty slots after a failed start** — the slot is reserved before the adapter is spawned, with the comment *"failed/cancelled starts remove the empty slot"*, but there is no `remove` on the `TcpListener::bind`, stdio-extraction, DAP-init or cancellation error paths. Self-healing via the `retain` at `debug_tools.rs:428-433`, but the entry counts toward the 16-session limit in between. | `debug_tools.rs:421-440`, `:601-615` |
| **D12** | **`McpHub::disconnect` removes the entry before shutdown, making the advised retry a no-op** — `connections.remove(id)` happens before `cancel()`. When the local server does not exit within 3 s the user is told *"try Disconnect again"*, but the second call finds nothing and returns `Ok(())` without attempting anything. | `connectors.rs:1073-1092`; `local_mcp_process.rs:21-36` |
| **D13** | **`request_stop` silently drops the stop request when the control channel is full** — the channel has capacity 8 and `Control::Write` shares it; `try_send(Control::Stop)` failure is discarded, so the process keeps running. `ProcessRegistry::stop` handles the same condition explicitly and returns an error — `request_stop` should too. | `process_tools.rs:462-467`, `:404`, `:598-615` |
| **D14** | **`process_stop` can report "outcome unknown" for a process that was already killed** — `supervise` terminates the child first, but the `stop_reply` oneshot is only sent at the very end, after a **blocking** full recursive workspace walk (`development_tools::freshness` + `source_inventory::inventory`) plus up to 100 × 10 ms of barrier polling and a SQLite write. `ProcessRegistry::stop` waits only 5 s, so on a large workspace the tool reports an unknown outcome and tells the model not to retry. | `process_tools.rs:711-758`; `verification_observer.rs:36-115` |
| **D15** | **Browser session scratch directories survive a crash** — `automation.rs:134-139` creates a `TempDir` under `<app_data>/automation/sessions/browser-XXXXXX` (a Chromium user-data directory, typically tens of MB). It is removed when the session is dropped, but `install_recovery::clean` only sweeps `models` and `runtimes` (`lib.rs:273`). | `automation.rs:134-139`; `lib.rs:273-279` |

---

## PART V — Systemic findings

These are not single bugs; they are the reason the bug count is high.

### S1 — Four tool-registry mechanisms, four approval mechanisms, two workspace-edit engines, fourteen HTTP policies

The 2026-10-21 harness audit counted these; they are confirmed and are the
root cause of C1, C3, C4, C9, C14, D7. The `harness::registry()` returns
`Vec<ToolSpec>` (`harness.rs:121`) but 17 `pub fn registry` exist across the
backend; dispatch lives in `harness::execute_inner` plus four separate
PTC/subagent paths. The approval gate is re-implemented in `chat.rs:3539-3660`,
`ptc.rs:710-717`, `ptc.rs:1096-1143`, `subagents.rs:1251`, `subagents.rs:1986`
— with A4/C4 showing the copies have already drifted. Workspace editing
exists in `workspace.rs` and `workspace_patch.rs`.

**The pattern to copy** is OpenCode's: one registry, one gate, one dispatcher.
The `ToolSpec` type added in-flight (`tool_spec.rs`) is the right foundation —
it needs to become *the* registry type, not one of five.

### S2 — The tool contract is now visible but barely enforced

`ToolSpec::with_output` is wired into dispatch correctly
(`harness.rs:465-471` → `tool_spec.rs:60-70`) — but only **5 of ~125 tools**
declare an output contract (`formatter.rs`, `hooks.rs`, `sandbox.rs` ×2,
`skills.rs` ×2). `tool_contract_regressions.rs:441-499` asserts every declared
contract names a real tool. This is in-flight work; the mechanism is right and
the coverage is the remaining work.

### S3 — The store is a single global `Mutex<Store>` over one SQLite file

`AppState::database()` (`lib.rs:170-174`) hands out a `std::sync::MutexGuard`.
Positively: there is **no** `MutexGuard` held across an `.await` anywhere in
the backend (verified by a scope-aware scanner over all 140 files and by
`cargo clippy -- -W clippy::await_holding_lock`, both clean) — because
`MutexGuard` is `!Send` and every async path is `Send`-bounded. That is a
correctness property worth preserving explicitly, and it is currently a
property of the type system rather than a stated invariant.

Negatively: 13 sites open a **second connection** to the same file (B1), the
`session_events` table has no unique constraint on `(conversation_id, seq)`
(only safe because `next_session_event_seq` is reached through the single
`AppState.store` mutex — a second connection breaks that), and the
`execute_batch` schema creation is **not atomic** (rusqlite 0.40.2 steps each
statement individually — verified in the vendored source).

### S4 — Resources are owned by registries, but the app has no teardown contract

This is the same class of problem as S1, one layer down. The backend has
**eight** owner-scoped registries — processes, terminals, PTY sessions,
debuggers, language servers, HTTP sessions, verification observers, MCP
connectors — and each one has a *per-item* close/stop API. **None of them has
an owner-scoped or global teardown.** Consequently:

* app exit leaves every model-spawned child running (A8), because Tauri exits
  via `std::process::exit` and no destructor runs;
* deleting a conversation leaves its children running (C20), because only the
  automation session has `stop_conversation`;
* `Manager::shutdown()` can orphan an LSP session (C22) because the map entry
  is cleared even when the session could not be taken;
* the artifact table has no retention policy (C21), and the startup collector
  for download partials recognises a filename pattern the producer never emits
  (A9).

**The pattern to copy.** Every registry should expose exactly two extra
operations: `remove_owned(owner)` and `close_all()`. Then a single
`AppState::teardown()` at exit, and a single `delete_conversation` cleanup,
close everything — and the per-item APIs stay as they are.

### S5 — The "no P0/P1" security result is real, and should be protected

The security audit found **no P0 and no P1**, and the checked-and-clean list is
substantial: cap-std is used consistently with `open_dir_nofollow` +
`FollowSymlinks::No` (no TOCTOU); there is **no** `sh -c` interpolation of
model data anywhere; interpreter code goes on stdin to a fixed driver and
PowerShell via `[scriptblock]::Create`; the environment is `env_clear()`-ed and
re-populated from a fixed allow-list; `web_fetch` and the research worker both
pin DNS and re-vet every redirect hop; the approval gate is re-derived on
every path; "trusted read" was checked entry by entry and none of them is a
write or exec privilege; a malicious MCP server cannot make a tool a trusted
read (`trusted_read` is decided by the backend, `connectors.rs:402-420`);
skills are SHA-256-pinned from a bundled lock file, not downloaded at runtime;
model download paths cannot escape managed storage; `capabilities::dump_config`
returns no secrets.

**These are load-bearing. Any refactor of the tool registry, the gate or the
transport must keep them.**

---

## PART VI — Comparison with other harnesses

| Layer | localLM | OpenCode V2 | Claude Code | Codex CLI |
|---|---|---|---|---|
| **Permission model** | 3 modes + a "trusted read" allowlist | ordered allow/ask/deny rules, wildcards, **last match wins**, per-agent overrides, `external_directory` + `doom_loop` guards | rule syntax + gitignore anchors + permission modes | sandbox / approval modes |
| **Tool contract** | 3-tuple; output contracts just landing (5/125) | 6-tuple (name, description, input, **output**, options, execute) | internal | — |
| **Extensibility** | none (frozen at compile time) | 16 plugin transform domains + RPC + custom tools + npm plugins | mods (`cmd.addTool`) | — |
| **Undo / rewind** | none | snapshot (git object DB) | `/rewind`, 100 checkpoints + retention sweep | — |
| **Formatter on write** | none (in flight) | 26 built-ins + config | hooks | — |
| **Instructions** | filename filter | `AGENTS.md` hierarchy + live reload | `CLAUDE.md` hierarchy | `AGENTS.md` |
| **Run serialization** | one **global** mutex for the whole app (C17) | per-key `SessionRunCoordinator` with a doorbell protocol | per-session | per-session |
| **Session persistence** | hand-rolled SQLite, no migration ledger (C15) | Drizzle ORM, typed schema | JSONL transcripts | JSONL + SQLite |
| **Structured errors** | none — all `String` (C13) | Effect tagged errors | typed | typed |
| **Shared transport policy** | 34 ad-hoc clients (C14) | shared | shared | shared |
| **Crash resilience** | `panic = "abort"`, Windows-only log (C12) | — | — | — |
| **Child-process lifetime** | survives app exit (A8) | — | — | — |
| **MCP** | rmcp, stdio + streamable HTTP, bounded | full | full | full |
| **Tool result bounding** | artifact-captured, 512–8192 B, lifecycle handles preserved (except `shell_run`, C2) | — | — | — |

### Where localLM is **ahead**

* **Tool breadth** — ~125 model-facing tools vs OpenCode's 26, including
  debugger, OpenAPI, database rehearsal, benchmarks, impact analysis, browser
  and computer-use domains that the competitors do not have at all.
* **Result bounding with lifecycle preservation** — `artifacts::bound_tool_result`
  keeps `processId`, `proposalId` and `stagedTree` structured while shrinking
  the model-visible excerpt. This is a real design win.
* **Boundary correctness** — see S4. The cap-std / pinned-DNS / envelope /
  vault discipline is, in several places, better than the competitors'.
* **Provider capability modelling** — `provider_capabilities.rs` binds
  tool-support controls to an **endpoint fingerprint** so a capability record
  self-invalidates when the base URL, format or model changes, and refuses to
  infer optional capabilities from a model family. This is more careful than
  most published harnesses.

### Where localLM is **behind**, and the fix

| Gap | Competitor mechanism | Effort |
|---|---|---|
| Rule-based permissions | OpenCode's ordered wildcard rules, last-match-wins, per-agent override, `external_directory` + `doom_loop` guards | **M** — the module already exists but is unwired (D7) |
| Output schemas on every tool | OpenCode's `output` field on all 26 tools | **M** — mechanism landed, 120 declarations to go (S2) |
| Plugin surface | OpenCode's 16 transform domains + custom tools; Claude Code's mods | **L** |
| Undo / rewind | Claude Code's `/rewind` with 100 checkpoints; OpenCode's git-object snapshot | **M** |
| Formatter-on-write | OpenCode's 26 built-ins gated on project detection | **S** — `formatter.rs` already exists |
| Instruction hierarchy + live reload | OpenCode / Claude Code `AGENTS.md` / `CLAUDE.md` hierarchy | **S** — `instructions.rs` already exists |
| Per-session run coordination | OpenCode's `SessionRunCoordinator` | **M** |
| Structured errors | Effect tagged errors | **M** |
| One shared HTTP transport policy | shared clients | **S** |
| Schema migration ledger | Drizzle / `user_version` | **S** |

**The pattern.** OpenCode and Claude Code both treat the **permission engine,
the tool contract, and the session store** as load-bearing infrastructure with
one implementation and a testable seam. localLM treats them as ~129 loosely
coupled modules. That is the structural cause of the 36 findings, and it is why
A4, C4, C7, S1 all describe the *same* class of bug — a copy of a gate that
drifted.

---

## PART VII — Remediation plan

### Tier 0 — correctness (do first; everything else assumes these)

| # | Change | File | Effort |
|---|---|---|---|
| 0.1 | Move the recovery `UPDATE`s out of `Store::initialize` into a one-shot startup path | `store.rs:442-443`, `presentation_store.rs:22`, `lib.rs:263-265` | **S** |
| 0.2 | Add `Store::open_shared(path)` (no recovery) and route the 13 live-DB sites through it, or through `AppState::database()` | `chat.rs:1882,1919`, `lsp_tools.rs:570,587,841`, `subagents.rs:109,1052,1649,1707`, `development_tools.rs:28,32`, `verification_observer.rs:112` | **M** |
| 0.3 | Regression test: native round → `open_shared` → settle → `native_tool_rounds` intact | `tool_contract_regressions.rs` | **S** |
| 0.4 | Fix the compaction retention window over the history region; make `compact_model_messages` honour `compaction_keep_last` | `compaction.rs:142-247` | **M** |
| 0.5 | Stop `mem::take`ing the calls on a finalizing round; keep them, emit one tool result per call, and replace `unwrap_or_default()` with `?` | `chat.rs:3231-3233`, `:3281-3282` | **S** |
| 0.6 | Charge the parent's budget cell and consult the parent's `turn_blocked` set in `call_child_tool` | `subagents.rs:1420-1450` | **S** |
| 0.7 | Remove the model-supplied `docker_exec` image (or validate it and pass it after `--`) | `harness.rs:978-981`, `sandbox.rs:60-65` | **XS** |
| 0.8 | Add `PR_SET_PDEATHSIG` to `managed_spawn` (mirror `runtime.rs:314-325`) and tear down `processes` / `terminals` / `debuggers` / `language_servers` in the `RunEvent::Exit` handler | `execution.rs:344-357`, `lib.rs:494-508` | **S** |
| 0.9 | Make the download partial name match the startup collector, or teach `install_recovery::clean` to remove `*.part` | `download.rs:125-132`, `install_recovery.rs:49-61` | **S** |

### Tier 1 — build and process (nothing else matters until these are green)

| # | Change | Effort |
|---|---|---|
| 1.1 | Add a CI `backend` job: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo check --all-targets`, `cargo test --lib` | **S** |
| 1.2 | Add a CI `frontend` job: `npm run typecheck`, `npm test` | **XS** |
| 1.3 | Make `cargo clippy --all-targets` green (it currently fails with hard errors) | **M** |
| 1.4 | Write the crash log to a platform-correct path with a size cap; consider `panic = "unwind"` | **S** |
| 1.5 | Update `AGENTS.md` / `docs/IMPLEMENTATION_PLAN_HARNESS.md` test-count baselines (699 → 836, 646 → 784) | **XS** |

### Tier 2 — concurrency and resource safety

| # | Change | Effort |
|---|---|---|
| 2.1 | Never hold the connectors mutex across network I/O (startup restore + connect) | **M** |
| 2.2 | Never hold the skills mutex across a download — clone the manager out first | **XS** |
| 2.3 | Make `SubagentRegistry` poison-tolerant (11 × `expect`) | **XS** |
| 2.4 | `tokio::process::Command` (or `spawn_blocking`) for the `git log` in `memory.rs` | **XS** |
| 2.5 | `spawn_blocking` for vault / Secret Service I/O | **S** |
| 2.6 | Transaction-wrap `save_todos` and `fork_session_events` | **S** |
| 2.7 | Spawn the PTY before taking the `pty_sessions` lock | **S** |
| 2.8 | Delete the redundant per-call `ensure_table` DDL | **XS** |
| 2.9 | Stop `process_start` / `terminal_create` / debugger children when their conversation is deleted | **M** |
| 2.10 | Add an artifact byte budget / retention sweep | **M** |
| 2.11 | Make `Manager::shutdown()` await or kill in-flight LSP sessions instead of orphaning them | **M** |
| 2.12 | Send `stop_reply` right after `terminate()`; move the blocking workspace walk off the tokio worker | **S** |
| 2.13 | Sweep `automation/sessions/browser-*` at startup | **XS** |
| 2.14 | `request_stop` must report a full control channel instead of dropping the stop | **XS** |
| 2.15 | `McpHub::disconnect`: cancel first, remove on success | **XS** |

### Tier 3 — contract correctness

| # | Change | Effort |
|---|---|---|
| 3.1 | Implement the `tool_invoke` arm (or resolve the wrapper in the PTC path) | **S** |
| 3.2 | Add `shell_run` to `lifecycle_excerpt` and surface its `status` | **XS** |
| 3.3 | Normalize the `finish` result at the finalizing boundary | **S** |
| 3.4 | Add the capability-kill re-check to PTC `run_parallel` | **XS** |
| 3.5 | Charge the budget per query/URL, not per tool name | **S** |
| 3.6 | Map Anthropic `refusal` / `pause_turn` / `model_context_window_exceeded` | **S** |
| 3.7 | Replace (not append) `id` / `name` in tool-call deltas | **XS** |
| 3.8 | Make MCP scope ids globally unique | **S** |
| 3.9 | Re-check the final envelope size against the budget | **XS** |

### Tier 4 — security hardening

| # | Change | Effort |
|---|---|---|
| 4.1 | Reuse `sandbox::vet_fetch_target` in `api_tools`, `openapi_tools` and the browser tool; add `.no_proxy()` + pinning there | **M** |
| 4.2 | Move the Google API key into the vault; return a presence flag | **S** |
| 4.3 | Wire `permission_rules` deliberately (assert the "settings-only origin" invariant, and stop `Allow` rules widening past the access mode) **or** delete the file | **M** |
| 4.4 | Replace `opener:allow-open-path` with a workspace-constrained Rust command | **S** |
| 4.5 | Document that `bash.environment` / `python.environment` / LSP env are cleartext, or move them to the vault | **S** |

### Tier 5 — structural (this is where the competitor gap closes)

| # | Change | Effort |
|---|---|---|
| 5.1 | One `http_client` module with named, policy-encoding constructors; migrate the 34 sites | **M** |
| 5.2 | A typed error enum per domain with a stable `code`; serialize `{code, kind, message}` | **L** |
| 5.3 | `PRAGMA user_version` migration ledger | **M** |
| 5.4 | Extract the round loop from `send_message_inner`; move the test module to `chat/tests.rs` | **M** |
| 5.5 | Replace the global `state.operation` mutex with a per-conversation coordinator | **L** |
| 5.6 | Make `ToolSpec` *the* registry type; give every tool an output contract | **L** |
| 5.7 | One approval gate, one dispatcher — collapse the four copies | **L** |

**Do Tier 0 and Tier 1 first.** Tier 0 is seven changes, mostly small, and two
of them (B1, A1) are silent data loss. Tier 1 is what stops the next silent
break from reaching a user.

---

## PART VIII — What is already right (do not regress)

These were verified and should be treated as regression-protected invariants:

* **Path handling** — cap-std with `open_dir_nofollow` + `FollowSymlinks::No`
  at every level; no TOCTOU; absolute paths, `..`, `\`, `:`, UNC/ADS, control
  characters, trailing dot/space and Windows device names rejected; the
  credential deny-list applied on every model-facing path.
* **Command execution** — no `sh -c` interpolation of model data anywhere; the
  shell string is a **single argv element**; interpreter code goes on stdin;
  the approved-file path re-hashes the source immediately before launch; the
  environment is `env_clear()`-ed and re-allowlisted.
* **SSRF on the researched path** — resolve once, pin with `resolve_to_addrs`,
  `.no_proxy()`, re-vet every redirect hop, per-hop fresh client. The Node
  research worker has an independent equivalent.
* **Credential vault** — AES-256-GCM, key in the OS credential store, credential
  id bound as AAD, ids restricted to `[a-z0-9-]`, size-capped, atomic
  temp+rename writes, presence checks that do not unlock the store.
* **Approval** — RAII guard removes the pending sender on every exit path;
  decisions are single-use and id-bound; a denial blocks the rest of the turn;
  a cancelled run leaves no pending approval.
* **Tool-result envelope** — versioned, validated, idempotent normalization;
  success and error share one shape; `error.retryable` is only ever true when
  `sideEffects` is `none` / `rolled_back`; bounded results stay valid
  envelopes and downgrade `success → partial`.
* **Tool discovery** — the allowlist is enforced **on invoke**, not just on
  search; recursive wrappers and `finish` / `update_context` / `ptc_run` are
  refused as targets; the reserved `__locallm_mcp_` namespace is enforced
  against server-supplied names.
* **MCP protocol** — JSON-RPC framing via the pinned rmcp SDK; bounded
  pagination (≤64 pages, ≤512 tools, ≤2 MiB); 4 MiB stdio frame cap with
  correct cross-read accounting; stderr to `/dev/null`; `KillOnDrop` +
  process-group ownership; bounded shutdown budget; resource URIs must be
  exactly-listed or template-matching; elicitation is form-only, rejects
  credential-shaped fields, and is bound to the exact request UUID.
* **Provider capabilities** — endpoint-fingerprint binding; a capability
  record self-invalidates when the base URL, format or model changes;
  optional capabilities are never inferred from a model family.
* **Model downloads** — size-capped, SHA-256-pinned, and the destination is
  re-checked with `canonicalize(...).starts_with(&directory)`.
* **Skills** — SHA-256-pinned from a bundled lock file; not downloaded at
  runtime.
* **Streaming** — byte-wise UTF-8 across chunk boundaries, CR/LF/CRLF, split
  BOM, multi-line `data:` joining, 1 MiB event cap, truncated-stream detection;
  dense tool-index remapping; duplicate item/call-id rejection;
  `response.incomplete` → `"length"`.
* **No `MutexGuard` across `.await`** — verified by scope-aware scan and by
  `clippy::await_holding_lock`. Preserve this; it is a real correctness
  property.
* **No pipe deadlock** — `run_code` / `run_command` use
  `tokio::try_join!(write, read_output(stdout), read_output(stderr), wait)`,
  so both pipes drain concurrently while the exit status is awaited. Git
  output is drained to EOF on **both** pipes before `child.wait()`. Terminal
  and runtime-log drains keep reading after a write failure so the child's
  pipe never blocks. `read_output` is bounded (64 KiB per stream) and the
  error path kills the child.
* **Process kill semantics** — `ExecutionChild::drop` → `start_kill()`, and
  with `ProcessGroup::leader()` that is `killpg(pgid, SIGKILL)` on the **whole
  group**, not just the leader. `kill_on_drop(true)` is set on every spawn
  path; Windows additionally gets a `JobObject` that owns the whole tree.
  `terminate()` does `start_kill()` **then** `child.wait()` with a timeout, and
  Tokio's orphan queue reaps children whose handle is dropped — so no
  permanent zombies from the drop paths. `PendingStart` removes the registry
  entry when a start is cancelled before the id is delivered.
* **Downloads are atomic and verified** — bytes go to `<name>.part`, the
  SHA-256 is verified and pinned, the file is `sync_all()`-ed, and only then
  renamed into place. Size is bounded by the pinned asset size. Resume
  validates `Content-Range`; a server that ignores `Range` triggers a clean
  restart with a disk-space re-check. `https_only` with a redirect limit.
  Runtime staging uses a `TempDir` removed on every path, including
  cancellation.
* **Cancellation is clean** — a pending approval is cancelled by racing
  `cancellation.changed()` against the decision timeout, and both
  `state.approvals.remove()` and `PendingApproval::drop` clear the entry, so
  no approval is left pending. `model_install` / `runtime_install` use a
  `RunGuard` and a cancel watch channel, re-checked before every irreversible
  step. Daytona deliberately **continues** after a cancel so the created
  sandbox is recorded and cleaned up — a remote resource is never orphaned.
* **PTY terminals** — `kill()` uses the cloned `ChildKiller` (not the child
  mutex) so it cannot deadlock against the exit watcher, and on Unix it
  `killpg`s both the foreground group and the shell's own group. The
  scrollback is capped at 256 KiB and the IPC backlog is acknowledged, so a
  slow UI cannot grow memory without limit. The two PTY threads are plain OS
  threads, not tokio workers.

---

## PART IX — Verification commands

```bash
# Backend (must be green before any release)
cd src-tauri
cargo fmt --check
cargo clippy --lib --all-targets -- -D warnings
cargo check --lib --all-targets
cargo test --lib

# Frontend
npm run typecheck
npx vitest run

# Specific regressions this audit implies
#  B1: native round survives a second Store::open on the live db
#  A1: compaction keeps compaction_keep_last complete turns
#  A2: a finalizing round with a tool call keeps native/canonical call sets equal
#  A3: a child's search is charged to the parent's budget cell
```

**Environment limits observed in this audit.** TCP socket creation fails with
`Operation not permitted` in this sandbox, so the MCP HTTP transport, the
provider loopback and the A11/A12 fixtures could not be exercised live; those
areas were audited by source trace only. No Docker daemon was available, so
C8 was confirmed by code path rather than by execution. The P0 (B1) was
reproduced directly against the verbatim schema and triggers, which is
deterministic SQLite semantics and not environment-dependent.

---

## Appendix — counts

| Metric | Value |
|---|---|
| Rust source audited | 93,729 lines, 129 files |
| Rust test functions | 877 (`cargo test --lib` → 836 passed, 0 failed, 17 ignored) |
| Frontend tests | 114 files, 784 passed, 1 skipped |
| Registered Tauri commands | 188 (172 invoked by the frontend) |
| Model-facing tools | ~125 across 7 presets (5 – 94 tools) |
| `Result<_, String>` sites | 179 (0 typed error enums) |
| `reqwest::Client` construction sites | 34 |
| Findings | 47 (1 P0, 9 P1, 22 P2, 15 P3) |

---

# PART X — Remediation status

Every finding has been addressed. The table records what changed, so the
report and the code can be read together.

## P0

| # | Fix |
|---|---|
| **B1** | `Store::initialize` no longer runs crash recovery. The recovery statements moved into `Store::recover_interrupted`, called **once** at startup (`lib.rs`), and `presentation_store::recover_interrupted` moved with them. `Store::open` is now safe on any connection, so the thirteen live-DB second connections can no longer flip a running tool's audit row out of `'streaming'` and trigger the `native_tool_round_edit` delete. Regression tests: `store::tests::a_second_connection_does_not_recover_an_in_flight_tool_audit` and `store::tests::explicit_startup_recovery_marks_abandoned_work_as_interrupted`. |

## P1

| # | Fix |
|---|---|
| **A1** | Compaction now plans against a **retention window** over the history region: `compaction::plan_compaction` counts user-message turn boundaries before the draft and keeps the last `keep` complete turns, cutting only on a user boundary so an assistant and its tool results are never split. `compaction_keep_last` is honoured for the first time. `chat::prepare_context` tries progressively smaller windows (`keep`, 4, 2, 1, 0) until the request fits, so the model keeps as much of the conversation as it can afford. The durable cutoff now uses the same window the model saw. |
| **A2** | The finalizing round no longer empties the call list. `ToolCalls::validated` validates without consuming, the suppressed round is recorded with its canonical `tool_calls` **and** its native output (so the Responses adapter's `native_calls == canonical` check holds) plus one explicit unknown-outcome tool result per call, and a malformed call is an error rather than a silently empty round. `arex::apply_control` reads the payload from inside the envelope. |
| **A3** | A child run id is bound to the parent's budget cell and replay-protection set (`subagents::bind_child_to_parent`), so a child — and any nested child — charges the same turn budget and obeys the same "never repeat an uncertain call" rule. `call_child_tool` charges the budget and consults the blocked set for both harness and MCP calls. |
| **A4** | The connector handshake runs **outside** the global hub lock: `McpHub::establish` performs DNS vetting, the HTTP request or the process spawn and the tool-catalog read without the lock, and only the bookkeeping takes it. Startup restore and manual connect both use it. |
| **A5** | `install_skill` clones the skills manager out of the lock before installing, so a download no longer freezes chat-turn assembly or every skill read. |
| **A6** | All eleven `SubagentRegistry` `.expect("subagent registry")` calls are poison-tolerant (`unwrap_or_else(\|e\| e.into_inner())`), matching every other lock in the backend. |
| **A7** | `memory::survey_repo` uses `tokio::process::Command` instead of a blocking `std::process::Command`. |
| **A8** | `managed_spawn` arms `PR_SET_PDEATHSIG` (the guard `runtime.rs` already used for llama-server), so the kernel kills every child when the app dies. The `RunEvent::Exit` handler now also tears down the process registry, the terminal registry, the debugger manager, the language servers and the connector sessions. |
| **A9** | The startup collector recognises the partial names the downloader actually writes (`<name>.part`), inside `<models>/` and inside a model's digest directory, guarded by the same non-symlink check. Regression test: `install_recovery::tests::reclaims_the_partial_names_the_downloader_actually_writes`. |

## P2

| # | Fix |
|---|---|
| **C1** | `tool_invoke` resolves to the real tool before dispatch, in `harness::execute` (so every path works) and in the PTC plan path. The model can now use the "search then invoke" pairing the instructions describe. |
| **C2** | `shell_run` joins the lifecycle-handle set, so a bounded result keeps its `processId` and its "still running" state (`processStatus`). |
| **C3** | The `finish` result is normalized into the `locallm.tool-envelope/1` envelope like every other tool result, and the envelope's `apply_control` reads the payload from inside it. The frontend reads both shapes. |
| **C4** | The PTC `parallel` path applies the same capability-kill re-check the sequential path does. |
| **C5** | The turn budget charges one unit per query or URL, not per call: `continuation::budget_units` counts the batch the arguments carry, so four queries cost four searches. |
| **C6** | Anthropic's `refusal` is a completed answer (its text is delivered), `pause_turn` is a resumable stop, and `model_context_window_exceeded` maps to the token limit. |
| **C7** | A repeated tool-call `id` or `name` is **replaced**, not appended; a genuine fragment is still appended, and a conflicting identity is rejected. |
| **C8** | The model can no longer choose the container image: the pinned user image is always used, a reference is validated against a strict pattern, and the image is passed after `--` so Docker can never read it as a flag. |
| **C9** | `sandbox::vet_fetch_target` and `sandbox::is_blocked_ip` are public and reused by `api_tools`, `openapi_tools`, the connector endpoints and the browser tool; those clients disable the proxy and pin the vetted address, and every redirect hop is re-vetted. |
| **C10** | `save_todos` and `fork_session_events` are each one transaction, so a partial write can no longer leave a half-written checklist or a half-forked conversation. |
| **C11** | `.github/workflows/ci.yml` runs the backend gate (`cargo clippy --lib --all-targets -- -D warnings`, `cargo check --lib --all-targets`, `cargo test --lib`) and the frontend gate (`npm run typecheck`, `npx vitest run`) on every push and pull request. |
| **C12** | The crash log is written to a platform-correct location — `%LOCALAPPDATA%` on Windows, `~/Library/Logs/LocalLM` on macOS, `$XDG_STATE_HOME/locallm` on Linux — instead of Windows only. |
| **C13** | A new `error` module defines `ErrorKind` and `AppError`. The tool envelope's error object now carries a stable `kind` beside the existing `code` and `message` (additive, so nothing that reads them changes), and every failure code maps to a kind. |
| **C14** | A shared `http_client` module owns the outbound HTTP policy; seven call sites now name the policy they want instead of building their own client. |
| **C15** | The store records `PRAGMA user_version`, refuses to open a database written by a **newer** build, and the additive steps are gated on the recorded version. |
| **C16** | `send_message_inner` went from **1,711 lines to 378**: the round loop is now `TurnRounds::run`, a named unit with its state in one struct, and the call site reads as the turn's steps. |
| **C17** | The global operation mutex became an `OperationGate`: a model turn holds a **shared** lease and an exclusive operation holds the **exclusive** lease, so several conversations can run at once while a provider edit or credential change is still refused during any turn. A per-conversation `TurnLocks` keeps one turn at a time per conversation. The 57 existing call sites keep their exact API and messages. |
| **C18** | The vault's keyring round trips (a blocking D-Bus call) moved off the tokio workers: `Vault::load_blocking` / `save_blocking` / `clear_blocking` run them in `spawn_blocking`, and the connector, Daytona and provider paths use them. |
| **C19** | The artifacts table has a per-conversation byte budget; the oldest artifacts are evicted when a conversation passes it, and the newest is never evicted. |
| **C20** | Deleting a conversation now stops the processes, terminal shells and debugger sessions it owns, so no child is left with no reachable handle. |
| **C21** | (Merged with C19.) |
| **C22** | `Manager::shutdown` no longer orphans a live LSP session: the removed slots are kept, and a session whose lock was busy is awaited, taken and shut down, so its process dies and its semaphore permit is returned. |

## P3

| # | Fix |
|---|---|
| **D1** | MCP input scope ids come from a process-wide sequence, so one connector's cleanup can no longer cancel another connector's pending form. |
| **D2** | The bounded envelope is re-checked against the budget after the envelope overhead is added, and the inner object is shrunk until the whole envelope fits. |
| **D3** | (Merged with C15.) |
| **D4** | The web-search cache is a real bounded LRU with an insertion order, not a full flush: one heavy run no longer discards every other conversation's cached evidence. |
| **D5** | The redundant per-access `CREATE TABLE IF NOT EXISTS` for research tasks is gone; the schema batch already creates it, and the DDL was taking SQLite's write lock on the tool-dispatch path. |
| **D6** | `terminal_open` creates the PTY **before** taking the sessions lock, so the lock is not held across a process spawn and two thread spawns. |
| **D7** | The rule engine is wired: `permission_rules` is declared, its commands are registered, and the approval gate consults it. |
| **D8** | `opener:allow-open-path` is gone; "open in external editor" goes through a Rust command that constrains the path to the selected workspace. |
| **D9** | (Fixed by B1: a live run is no longer reported as cancelled.) |
| **D10** | The verified download is installed with `hard_link`, which fails with `AlreadyExists` rather than silently replacing the destination — the "an existing destination is preserved" guarantee is now true. |
| **D11** | A failed or cancelled debugger start releases its reservation, so it no longer counts against the session limit. |
| **D12** | A connector whose shutdown was signalled but never confirmed is recorded, and a second Disconnect reports that state instead of a false success. |
| **D13** | `request_stop` reports a full control channel instead of silently dropping the stop. |
| **D14** | The stop reply is sent as soon as the child is terminated, and the blocking workspace walk moved off the tokio worker, so `process_stop` no longer reports "outcome unknown" for a process that was already killed. |
| **D15** | Browser session scratch directories are swept at startup. |

## Verification

| Gate | Result |
|---|---|
| `cargo check --lib --all-targets` | clean |
| `cargo clippy --lib --all-targets -- -D warnings` | **0 warnings** (was 185) |
| `cargo test --lib` | **968 passed, 0 failed, 17 ignored** |
| `npm run typecheck` | clean |
| `npx vitest run` | **770 passed, 1 skipped** |

`cargo fmt --check` is deliberately **not** a gate: the backend uses a dense
house style and was never rustfmt-formatted (248 diffs before this work). CI
gates on clippy instead, which catches the lints that matter.
