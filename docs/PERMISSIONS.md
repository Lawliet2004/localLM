# Tool permissions

Each conversation has a **Permissions** selector above the chat:

| Mode | Behavior |
| --- | --- |
| Ask for approval | Every selected tool action requires an Allow once or Deny decision. |
| Auto-approve reads | Built-in workspace `read_file` and `list_files` run automatically. Writes, local code, skill package reads and all connector calls still ask. |
| Full access | Selected enabled tools run automatically, including local code and connector actions, except stricter per-tool “always ask” overrides. |

Denying a request blocks further tool use for the remainder of that turn, including other calls in the same batch. The model may still give a written response. A new message starts a fresh turn.

## Permission rules

The access mode decides the calls that no rule matched. A rule is your explicit instruction and is evaluated **first**:

| Effect | Meaning |
| --- | --- |
| `allow` | The call runs without a prompt. |
| `ask` | The call always prompts, even under Full access. |
| `deny` | The call is refused, even under Full access. |

Rules are stored under **Settings → Permission rules**. Each rule pairs an action with a resource:

| Action | Resource | Example |
| --- | --- | --- |
| `command` | A shell command prefix | `cargo test*` allows `cargo test` and `cargo test --all`. A trailing ` *` also matches the bare command. |
| `path` | A workspace path | `src/**` matches every file under `src`; `**/*.env` matches a `.env` at any depth. |
| `tool` | A tool name | `shell_run*` covers the shell family. |
| `host` | A web host | `*.example.com` matches `api.example.com` but not `example.com`. |

The **last matching rule wins**. A rule that names a tool which always asks for approval does not override that: the tool still prompts.

Rules can only ever come from your saved settings. Model output can never create, widen or remove a rule.

New conversations inherit your last explicitly selected permission mode and tools, including after restarting the app. A saved conversation keeps its own mode. With no saved preference, the default is Ask for approval. On upgrade, older remembered tool defaults inherit the most recently saved conversation's mode once. Changing modes during generation is disabled: stop the response before changing permissions.

Successfully connected services are remembered and reconnected at startup. Disconnecting keeps a service off on later restarts. Reconnection uses existing credentials without opening a sign-in flow; expired authorization or an unavailable service leaves it disconnected with an explanation in Connectors. Active skill selections also persist across chats and restarts.

Full access changes approval behavior, not tool selection. It does not install tools, enable connectors, supply account credentials, or remove schema checks, workspace file boundaries, output limits, cancellation and timeouts. Local execution runs with your account's filesystem and network permissions and is not sandboxed. Bash and Python each have an optional “always ask” policy that requires code review even in Full access. Python file approval snapshots the exact source; a changed file is rejected before execution. Approval shows the effective working directory. Unix memory limits bound each process's address space, not its permissions or total descendant memory.

Auto-approve reads uses an explicit native policy, not an AI reviewer. Tool descriptions and server-provided claims do not grant automatic approval. Skills and model messages cannot change the conversation's permission mode.

Every tool audit records its permission mode, allow/deny decision, authorization source, arguments and result. An automatically approved operation can still fail; approval is not a success guarantee.

Activating one or more skills enables one shared `skills_read_file` tool and adds their package file lists to the model's instructions. This consumes one of the 32 tool slots. It can only read verified text files from skills active at the start of that turn. It cannot execute scripts. Ask and Auto-approve reads both prompt for package reads; Full access allows them without prompts. Deactivate all skills to remove this tool.

Daytona cloud code is separately selectable after saving a key and consumes one tool slot. Ask and Auto-approve reads both require approval before sandbox creation. The approval shows the submitted code and explains that cloud usage may incur charges. Full access permits selected Daytona calls without prompts. Workspace files are not uploaded automatically. Credential changes are refused during a model operation so the selected credential cannot change while approval is pending.

## Browser and computer automation

Browser and Computer are explicit, disabled-by-default capabilities configured in Browser use and Computer use settings and selected per conversation. Node.js interpreter setup is in Execution. Every action, including screenshot and pointer reads, asks under Ask and Auto-approve reads. Full access runs selected actions without prompts. They use the ordinary tool audit and cancellation path. Screenshots are conversation-owned image artifacts and may enter a configured vision model's input. Browser profiles are isolated per conversation; native desktop actions affect the primary monitor and focused application. See [automation setup and platform limits](AUTOMATION.md).
