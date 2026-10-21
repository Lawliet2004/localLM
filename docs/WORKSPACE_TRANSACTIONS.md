# Native workspace file transactions

The Workspace source now advertises six additional tools using the existing `AgentTool` interface and strict JSON parameter schemas:

| Model alias | Purpose |
|---|---|
| `workspace_apply_patch` | Batch 1–16 file edits, creations and deletions. |
| `workspace_copy_file` | Copy a hash-pinned file to a new destination. |
| `workspace_move_file` | Recoverable copy/delete move to a new destination. |
| `workspace_delete_file` | Delete a hash-pinned regular file. |
| `workspace_file_info` | Read SHA-256, byte size and text/binary classification. |
| `workspace_recover_patch` | List pending transactions, roll back prepared transactions or finish committed cleanup. |

All require an explicitly selected workspace source and an applicable preset. Capability disable controls still apply. `file_info` is a trusted read; the other five follow the existing write approval rules. No new package is required.

Existing files require the SHA-256 from a fresh `workspace_read_file` or `workspace_file_info`. A destination must be absent and its parent directory must already exist. Files are limited to 1 MiB; a transaction's combined original and replacement bytes are limited to 8 MiB. Native transactions reject links, hard-linked mutation targets, credential paths and internal transaction paths.

For example, after reading `src/message.txt` and obtaining its hash:

```json
{
  "name": "workspace_apply_patch",
  "arguments": {
    "changes": [
      {
        "kind": "edit",
        "path": "src/message.txt",
        "expected_sha256": "<64-character hash from the fresh read>",
        "replacements": [{"old_text": "Hello", "new_text": "Welcome"}]
      },
      {"kind": "create", "path": "src/added.txt", "content": "New file\n"}
    ]
  }
}
```

Each replacement must match exactly once in the original text. Replacements within one file cannot overlap. Paths must be distinct, with one exception: reviewed case-only delete/create pairs are coalesced into a single rename journal entry. Any preflight conflict leaves all original project files unchanged.

The backend returns a transaction ID, an explicit status, per-file hashes and bounded text diffs. Status is `committed`, `rolled_back`, `not_started`, or `unknown`. Failed outcomes carry `isError` and a message. `recoveryRequired`, cleanup errors and conflict paths describe unresolved work rather than silently claiming success. Binary copy/move/delete is supported within the same size limits; text edits require UTF-8.

Before changing project files, the implementation writes original copies and replacement stages in each target's parent directory and records a prepared journal in `.locallm-transactions`. It syncs file data and, on Unix, directory metadata. Replacing an existing file uses a same-parent rename. Creation uses a no-clobber hard link whose support is checked during preparation. Original permissions are preserved. Only after all changes are synced is the journal marked committed.

Native Workspace calls and editor operations share a root-scoped lock. Replacement is atomic **per file**; arbitrary other applications can observe transitions between files and do not participate in this lock. Hash checks are optimistic conflict detection, with a final check immediately before mutation. They are not an operating-system compare-and-swap guarantee against an external writer racing that final check.

On a failed commit, the tool attempts rollback and preserves divergent external edits. Unconfirmed rollback returns `unknown` and keeps recovery material. After interruption/restart, ordinary workspace/editor reads or writes encountering a journal fail with an actionable recovery instruction. A read never performs write recovery automatically.

```json
{"name":"workspace_recover_patch","arguments":{}}
```

This lists transaction IDs. To recover one:

```json
{"name":"workspace_recover_patch","arguments":{"transaction_id":"<32 lowercase hexadecimal characters>"}}
```

Prepared transactions roll back; committed transactions finish cleanup. Recovery validates the manifest, paths and original backup hashes before restoring files. Divergent files remain untouched, are listed as conflicts, and keep the transaction pending. Internal journals/stages are unavailable through normal model file tools or editor mutations.

Implementation: `src-tauri/src/workspace_patch.rs`; registration: `Workspace::tools` in `workspace.rs`. Eleven focused tests exercise conflict preflight, real commit failure/rollback, interrupted prepared and committed recovery, Unicode/CRLF/permissions, corrupt recovery material, concurrent edits, binary operations, path/link/size limits, shared locks and parsed model-call dispatch. A separate editor test checks pending-transaction guards. The implementation remains subject to the full plan's SWE-2 Max review and final acceptance gates.

## Case-only semantic renames

Reviewed semantic proposals can rename a file by case alone and then edit the
new path in the same transaction. The journal records both names and both content
hashes. Exact directory-entry spelling plus opened-file identity distinguishes
an alias on an insensitive filesystem from a second, distinct file on a sensitive
filesystem. A collision is refused even if the files have identical contents.
Changing parent-directory spelling is outside this file-only operation.

All targets and guards pass preflight before mutation. The original inode is
parked under an owned same-parent transaction name, then the destination spelling
is installed without clobbering. Both steps are journal-covered and synced.
Recovery handles preparation, the parked-only interval, installed destination,
commit failure and interruption during rollback. It restores the original name
and contents, or preserves divergent destination/parked contents as a conflict.
An external application holding the original open inode can still write during
this interval; divergence prevents recovery from silently overwriting that edit.
Permissions, no-follow opens, single-link identity and workspace containment
checks remain in use. This is optimistic conflict detection, not filesystem
isolation from unrelated writers.

The platform-neutral transaction tests cover both name directions, stale hashes,
distinct-file collisions, rename-plus-edit, failed commit, interrupted recovery
and external edits. Linux was executed locally; Windows/macOS native execution is
still an external acceptance requirement.
