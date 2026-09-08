# Skills

The Skills page installs the 13 presets recorded in `catalog/skills.lock.json`. Each package has an upstream repository, pinned revision, package-relative file list, expected byte sizes and SHA-256 hashes. Activation supplies its `SKILL.md` and file inventory to subsequent turns. Active skills are currently shared across conversations; the active set is captured when each turn starts.

## Package references

Activating any skill enables one shared `skills_read_file` tool. Its `skill_id` must belong to that turn's active set, and `path` must exactly match a file in the pinned package. Reads verify the complete file before returning text. Directory capabilities prevent junction or traversal escapes, and reads stop at the expected file size plus one byte to detect changes without unbounded allocation.

The tool accepts optional `start_line` (default 1) and `line_count` (default 200, maximum 500). It returns numbered UTF-8 lines, total line count, `has_more` and `next_line`. Each page contains at most 64 KiB of source text; a single oversized line fails explicitly. Binary files cannot be returned as text. The context preflight still applies before the model continues with the result.

The reader consumes one of the 32 tool slots regardless of active skill count. The chat tool picker displays it. Ask and Auto-approve reads require approval for package reads; Full access skips the prompt. The ordinary tool audit records arguments, authorization and results. Deactivate every skill to remove the reader.

Reading script source does not execute it or install dependencies. Dedicated package script execution, dependency reporting and package update workflows remain incomplete. Current local code execution is a separate selected tool and is not sandboxed.

## Evidence

`scripts/skill-reader-smoke.mjs` uses the native desktop application and real MiniCPM runtime to request the first three lines of the Jupyter notebook quality checklist. It compares returned lines to the verified package content, checks continuation metadata, checks authorization prompts in all three modes, and verifies the model's answer uses the reference. It restores active skills and generation preferences and deletes its temporary conversations.

Rust tests cover pagination, Unicode, oversized lines, invalid parameters, inactive/uninstalled packages, unknown paths, file tampering and a real Windows junction escape. Frontend tests verify the reader reserves a visible tool slot and prevents exceeding the limit.
