# Typed Git tools

The native registry exposes `git_status`, `git_diff`, `git_stage`, `git_unstage`, `git_commit`, `git_branch` and `git_history`. They call the existing `workspace_ui` Git backend directly, with argument vectors and literal pathspecs. Model arguments cannot choose a shell, executable, repository root, environment, force/reset flag or confirmation. No new dependency is added.

These tools are offered in Standard, Coding, Code and Creator when the workspace source is selected and Git is available. Minimal remains narrow. The `git_tools` capability depends on both `workspace_files` and `local_execution`: disabling any of these rejects stale direct, nested and detached calls. Configured Git hooks/filters can execute code, so disabling local execution also removes these external commands. Status, diff and history qualify for read approval but remain sequential because Git may refresh its index. Writes use the normal mutation policy. `git_commit` always requests user approval, including in Full Access and PTC; background children cannot bypass that setting.

A normal review sequence is:

```json
{"name":"git_stage","arguments":{"paths":["src/example.rs"]}}
{"name":"git_diff","arguments":{"kind":"staged"}}
{"name":"git_commit","arguments":{"message":"Fix example parsing","expected_tree":"<stagedTree from the reviewed diff>"}}
```

Only an unfiltered, untruncated staged diff at the repository root provides a `stagedTree` commit-review pin. The index is checked before and after the diff; committing rejects a stale pin. The existing backend also requires the repository root and explicit approval. Ordinary Git hooks and configuration still apply, except fsmonitor hooks are disabled on the shared runner so read-only status checks cannot invoke a project script. Nothing pushes changes remotely. Git commands can have side effects before reporting failure, especially hooks; an uncertain outcome must be inspected rather than automatically replayed.

`git_diff` accepts `kind:"working"|"staged"|"all"` and optional literal `paths`. Working compares disk to index, staged compares index to HEAD (or the empty tree before the first commit), and all compares tracked disk contents to HEAD. An unborn repository must use staged and working diffs separately for that last mode. Untracked files appear in status and are not implicitly staged or included in a normal Git diff.

Staging and unstaging require1–200 explicit paths. The backend rejects traversal and `.git` paths and disables pathspec expansion. Typed content operations additionally check credential-path policy, no-follow parent opens, regular files and the shared opened-handle hard-link checks. Symlink/submodule content operations require another explicitly authorized workflow. A filtered path inventory that exceeds its bound fails visibly rather than asserting it checked every path. Status and mutation responses omit unrestricted diff content; use `git_diff` for guarded content access.

Branch creation and switching are local and never force discards. Whole-repository branch/history/commit operations require the repository root so a subdirectory workspace cannot silently authorize changes across the rest of the repository. History returns at most50 entries, with `limitReached` when older commits may exist. `headVerified` and `historyState` distinguish verified history from an unborn or unavailable HEAD. Status exposes `incomplete` and warnings if its output or staged-tree identity is incomplete.

The shared Git runner drains both pipes to EOF, retains bounded prefixes, observes the exit status and applies its existing15-second timeout. Large stdout or stderr cannot turn a failed command into a success or deadlock a full pipe. Results then use the standard versioned tool envelope and artifact bounding. A large staged diff preserves its exact `stagedTree` in the compact native result and marks `reviewIncomplete:true`; read the saved artifact before claiming a complete review. A visible hash is not user approval.

Git remains an external process boundary. A pending native patch journal is rejected before launch. Tree and filesystem checks are optimistic; another process can change files or the index after a check. Git, shell commands, hooks and external applications do not participate in native workspace transaction isolation. These checks do not establish an operating-system atomic compare-and-swap across file review and commit. Windows runtime verification is still required before claiming platform parity.

Verification and acceptance status are tracked in `INFERENCE_TOOL_IMPLEMENTATION_PLAN.md`. Temporary-repository tests never stage or commit this project's working tree.
