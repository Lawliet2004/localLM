# Harness tools

The harness exposes its own capabilities to the model as ordinary tools. This
page is the map of what the model can reach, and why each one exists.

Everything here runs on the local machine, under the same permission and
capability rules as every other tool. Nothing grants access the user did not
already grant.

## Tool contracts

Every tool is declared with a name, a description, an **input schema**, and —
where the tool returns a structured result — an **output schema**. The input
schema is checked before the tool runs; the output schema is checked after it
returns, at the same boundary where the result becomes model-visible.

That second check matters: a tool that drifts from its documented shape fails
where it can be diagnosed, instead of confusing the model three calls later.
Tools without a declared output contract are unaffected — a contract is
opt-in, so a tool can ship before its result shape is stable.

Schemas that would not compile, or that name a tool that does not exist, are
caught by the test suite. The registry itself is one type
(`ToolSpec`), so a tool cannot be registered with a description and no schema.

## Files and edits

| Tool | What it does |
| --- | --- |
| `file_read` / `file_search` / `file_write` | Read, search and create workspace files. |
| `workspace_*` | The full workspace connector: listing, reading, editing, patching, notebooks, documents, images. |
| `format_paths` | Format files in place using the project's formatter. |

**Edit matching is tolerant.** An edit finds its target in three passes:
exact bytes, then typographic variants (smart quotes, en/em dashes, exotic
spaces), then trailing whitespace ignored per line. `replace_all` replaces
every occurrence instead of requiring a unique one. The replacement reuses the
line endings of the region it replaces, so a CRLF file stays CRLF and
untouched bytes are never rewritten.

The point is that a one-character slip by the model — a smart quote, a
trailing space, a CRLF — should not cost a three-tool round trip.

**Writes are formatted.** After a write, edit or patch, the project's
formatter runs on the file: `rustfmt`, `prettier`, `biome`, `gofmt`,
`clang-format`, `ruff`, `black`, `gofumpt`, `shfmt`. A formatter applies when
the file's final extension matches, the project uses it (`prettier` needs a
`prettier` dependency in the nearest `package.json`), and its binary is
installed. Formatting never fails a write: a broken toolchain is reported in
the tool result, and the file keeps the bytes it was given.

Every mutating tool still requires an explicit `expected_sha256` pin, and the
reported hash is the hash **after** formatting, so the next edit's pin lines
up with what a read returns.

## Code intelligence

`lsp_*` tools cover diagnostics, symbols, references, hover, implementations,
code actions, rename and server commands. Renames are staged as proposals and
applied separately, so a rename is reviewable before it happens.

`debugger_*` tools drive a stdio DAP adapter or an authenticated CodeLLDB
connection: start, inspect threads and scopes, set breakpoints, read events,
stop. Nothing in the reference harnesses offers a debugger.

## Verification and performance

`check_run` / `check_report` run a build or test suite and read back
structured evidence. `verification_*` binds a check to an explicit set of
source hashes. `benchmark_*`, `profile_capture`, `profile_inspect` and
`profile_compare` measure and compare. `change_impact` analyses what a change
touches, semantically or lexically.

## Web and research

`web_search`, `web_open`, `web_find`, `web_fetch_url` and `web_read` research
the web with bounded output; documents can be read back page by page.
`get_weather` reads current conditions and a forecast. The AREX research
preset swaps these for `search`, `visit`, `update_context` and `finish`.

## Orchestration

`subagent` delegates to a child with its own history, in the foreground or
in the background. `send_message` delivers a follow-up to a running child and
`interrupt_agent` stops one. `workflow_run` runs orchestrated stages and
`ralph_run` iterates toward a fixed objective.

`todo_write`, `todo_add` and `todo_update` keep a flat checklist. `task_*`
keeps a **dependency graph**: a task can depend on others, and a blocked task
cannot start until its dependencies are finished. `goal_set` records the
long-running objective.

`report_findings` records structured code-review findings — a file, what is
wrong, and the failure scenario that makes it matter — and `list_findings` and
`resolve_finding` manage them across the conversation.

## Managing the harness itself

`session_rename` renames the conversation. `session_move` moves it to another
directory. `worktree_list` lists the repository's worktrees, and
`worktree_enter` switches into one. `models` searches the models this
installation can use, so the model can turn a name the user mentioned into an
exact reference before handing it to a subagent.

`skill_list` and `skill_load` expose the installed skills to the model. A
skill's instructions are project text, not a permission grant.

## Learning the working style

`memory_teach` and `memory_recall` hold facts the user states outright.
`taste_recall` and `taste_teach` cover the other half: what the harness
learns from the decisions the user actually made. Every explicit approval is
recorded — accepted, rejected, or edited — and the resulting profile reports
which tools are accepted, which are rejected, and how often proposals are
edited before approval.

Auto-approved calls are deliberately **not** recorded: they reflect the
access mode, not the user's taste. Secrets are redacted before anything is
stored, and the signal log is bounded.

## Plugins

`plugins` are user-authored tools: a name, a description, an argument list, a
JSON Schema for the arguments, and a timeout. They are declared in JSON files
under the app's data directory and loaded at turn time.

A plugin runs with the user's privileges, so it is gated behind the same
capability as shell execution and it always asks for approval. The command is
an **argument list**, never a shell string — nothing the model supplies is
ever interpolated into a command line. A plugin that fails validation is
skipped and reported; one broken file never takes the catalog down.

See [plugin tools](PLUGINS.md).

## Undo

Before each model step the harness captures the workspace files the
conversation has already touched. A staged revert restores them byte for byte;
committing it tells you which files were restored, which were deleted, and
which had newer content that was overwritten. See [snapshots and undo](SNAPSHOTS.md).

## Project instructions

The workspace's `AGENTS.md` files are loaded into the system prompt as project
rules. They are read from the workspace, so they never grant access — they
say how work is done here. A file deeper in the tree can have its own
`AGENTS.md`; reading that file surfaces it, and the rules that changed
mid-conversation are reported to the model rather than silently swapped.

## Programmatic tool calling

`ptc_run` runs a bounded JSON program of tool calls: `let`, `call`, `if`,
bounded `for`, and `parallel`. Every call inside it is individually
policy-checked, audited and bounded, exactly as a direct call would be.
