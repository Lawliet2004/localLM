# Tool permissions

Each conversation has a **Permissions** selector above the chat:

| Mode | Behavior |
| --- | --- |
| Ask for approval | Every selected tool action requires an Allow once or Deny decision. |
| Auto-approve reads | Built-in workspace `read_file` and `list_files` run automatically. Writes, local code, skill package reads and all connector calls still ask. |
| Full access | All selected tools run without approval prompts, including local code and connector actions that may change external data. |

Denying a request blocks further tool use for the remainder of that turn, including other calls in the same batch. The model may still give a written response. A new message starts a fresh turn.

New conversations start in Ask for approval. A saved conversation remembers its own mode. Older saved conversations also default to Ask. Changing modes during generation is disabled: stop the response before changing permissions.

Full access changes approval behavior, not tool selection. It does not install tools, enable connectors, supply account credentials, or remove tool argument checks, workspace file boundaries, output limits, cancellation and timeouts. Local execution runs with your Windows account's filesystem and network permissions and is not sandboxed. In Full access it runs without a code-review prompt.

Auto-approve reads uses an explicit native policy, not an AI reviewer. Tool descriptions and server-provided claims do not grant automatic approval. Skills and model messages cannot change the conversation's permission mode.

Every tool audit records its permission mode, allow/deny decision, authorization source, arguments and result. An automatically approved operation can still fail; approval is not a success guarantee.

Activating one or more skills enables one shared `skills_read_file` tool and adds their package file lists to the model's instructions. This consumes one of the 32 tool slots. It can only read verified text files from skills active at the start of that turn. It cannot execute scripts. Ask and Auto-approve reads both prompt for package reads; Full access allows them without prompts. Deactivate all skills to remove this tool.

Daytona cloud code is separately selectable after saving a key and consumes one tool slot. Ask and Auto-approve reads both require approval before sandbox creation. The approval shows the submitted code and explains that cloud usage may incur charges. Full access permits selected Daytona calls without prompts. Workspace files are not uploaded automatically. Credential changes are refused during a model operation so the selected credential cannot change while approval is pending.
