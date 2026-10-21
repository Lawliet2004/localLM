# Plugin tools

A plugin is a tool you write, that the model can call. The built-in tool set
is compiled in; plugins are the extension point for the tools *you* need —
deploy, ticket management, a house script, a company API.

## Format

A plugin is one JSON file in the app's data directory:

```
<app data>/plugins/<name>.json
```

```json
{
  "name": "deploy",
  "description": "Deploy the current build to staging.",
  "command": ["bash", "scripts/deploy.sh"],
  "arguments": {
    "environment": {
      "type": "string",
      "description": "Target environment"
    }
  },
  "required": ["environment"],
  "timeout_seconds": 60
}
```

| Field | Rule |
| --- | --- |
| `name` | `^[a-z][a-z0-9_]{1,48}$`. Must not collide with a built-in tool. |
| `description` | What the tool does, written for the model. |
| `command` | **An argument list**, 1–32 entries. Never a shell string. |
| `arguments` | A JSON Schema for the arguments. |
| `required` | Names of keys in `arguments.properties`. |
| `timeout_seconds` | 1–600. |

A plugin file must be under 256 KiB. At most 64 plugins are loaded.

## Why the command is an argument list

Nothing the model supplies is ever interpolated into a command line. The
plugin's `command` is fixed by you; the model's arguments are appended as
separate `--key value` entries, so a value that contains a space, a quote or
a `;` is still one argument. A shell string would make every argument a
command.

## Safety

A plugin runs with **your privileges**, exactly like a shell tool. It is
therefore:

- gated behind the same capability as local execution — disabling shell
  execution disables plugins;
- **always** approved, even under Full access;
- recorded in the ordinary tool audit, with the same cancellation and
  timeout path.

A plugin that fails validation is skipped and reported in the plugin list;
one broken file never takes the tool catalog down.

## Managing plugins

**Settings → Plugin tools** lists the installed plugins and any load errors,
and creates or removes them.

Because the plugin directory is plain JSON, a plugin can also be written by
hand or checked into a repository and copied in.

## Limits

A plugin's result is `{"exit": number, "stdout": string, "stderr": string,
"truncated": boolean}`. Output is bounded at 64 KiB. A non-zero exit is a
**failed result**, not a crash — the model sees the exit code and the output.
A plugin that exceeds its timeout is killed and reported as a timeout.
