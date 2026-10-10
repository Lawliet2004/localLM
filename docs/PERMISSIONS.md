# Tool permissions

Each conversation has a **Permissions** selector above the chat:

| Mode | Behavior |
| --- | --- |
| Ask for approval | Every selected tool action requires an Allow once or Deny decision. |
| Auto-approve reads | Built-in workspace `read_file` and `list_files` run automatically. Writes, local code, skill package reads and all connector calls still ask. |
| Full access | All selected tools run without approval prompts, including local code and connector actions that may change external data. |

Denying a request blocks further tool use for the remainder of that turn, including other calls in the same batch. The model may still give a written response. A new message starts a fresh turn.

New conversations inherit your last explicitly selected permission mode and tools, including after restarting the app. A saved conversation keeps its own mode. With no saved preference, the default is Ask for approval. On upgrade, older remembered tool defaults inherit the most recently saved conversation's mode once. Changing modes during generation is disabled: stop the response before changing permissions.

Successfully connected services are remembered and reconnected at startup. Disconnecting keeps a service off on later restarts. Reconnection uses existing credentials without opening a sign-in flow; expired authorization or an unavailable service leaves it disconnected with an explanation in Connectors. Active skill selections also persist across chats and restarts.

Full access changes approval behavior, not tool selection. It does not install tools, enable connectors, supply account credentials, or remove tool argument checks, workspace file boundaries, output limits, cancellation and timeouts. Local execution is unsandboxed by default and runs with your Windows account's filesystem and network permissions; Settings → Execution can enforce an AppContainer sandbox confined to the workspace instead. In Full access it runs without a code-review prompt.

Auto-approve reads uses an explicit native policy, not an AI reviewer. Tool descriptions and server-provided claims do not grant automatic approval. Skills and model messages cannot change the conversation's permission mode.

Every tool audit records its permission mode, allow/deny decision, authorization source, arguments and result. An automatically approved operation can still fail; approval is not a success guarantee.

Activating one or more skills enables one shared `skills_read_file` tool and adds their package file lists to the model's instructions. This consumes one of the 32 tool slots. It can only read verified text files from skills active at the start of that turn. It cannot execute scripts. Ask and Auto-approve reads both prompt for package reads; Full access allows them without prompts. Deactivate all skills to remove this tool.

New Daytona cloud execution is unavailable. Historical cloud records remain readable, and cleanup credentials and retry actions are retained for unresolved resources. Docker execution is deferred; saved container preferences block execution until the user explicitly saves local execution settings.

# Task workspaces

**Isolate task** (composer) or the Review tab can give a conversation an isolated workspace: a git worktree of the repository when the workspace is a git repo, or a bounded snapshot copy (max 20,000 files / 500 MiB) otherwise. While it is active, every tool, terminal, execution, subagent and workflow resolves the conversation's workspace root to the isolated copy, so your files stay untouched until you apply the changes from the Review tab.

- The isolated copy carries your uncommitted and untracked changes into its own baseline commit. Files over 5 MiB, secret-like names (`.env`, `*.pem`, `*.key`, key files, `credentials*.json`), and generated folders (`node_modules`, `target`, `dist`, `.venv`, `__pycache__`) are never carried and are listed under exclusions.
- Checkpoints are commits on the isolated repo, taken automatically before shell/execution calls, before apply, and before a restore. Restoring a checkpoint rewinds the isolated copy only.
- Apply writes changes back file-by-file. If the source file changed since the task started it is reported as a conflict and skipped unless you choose "Use task version" or "Keep current file". Every overwritten or deleted file is first backed up under `task-recovery/` in the app data directory, so "Undo last apply" can restore it — but only if nothing newer has been written since. Undo restores files only. It cannot reverse messages sent, packages installed, or remote changes.
- Discard removes the worktree and its `localllm/task-*` branch (or deletes the snapshot). It never touches the source folder.
- One isolated task per conversation; it survives restarts. Creating one requires `git` on PATH — baselines and checkpoints are commits even in the snapshot copy.
- **Limits:** this is a review and recovery boundary, not a security sandbox. Code the agent executes keeps your full filesystem and network access. Applying changes cannot prove they are correct — review the diff first.
