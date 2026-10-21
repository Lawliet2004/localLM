# Snapshots and undo

The harness captures the workspace files a conversation has already touched
before each model step. That makes a bad edit recoverable without asking the
model to redo the work, and without the user running `git checkout` and
losing everything else the agent did.

## What is captured

The tracked paths come from the conversation's own tool audit: every path the
agent asked to read, write, edit, patch or move. The capture is therefore
bounded by what the conversation actually touched, not by the size of the
workspace.

A file above **2 MiB** is skipped and listed as skipped. A file that does not
exist is recorded as *absent*, so a restore deletes it again. A symbolic link
is never followed and never written through. Paths outside the workspace are
refused.

## Where the bytes live

Captured content never enters the workspace. It is stored in a private,
content-addressed blob directory next to the local database:

```
<app data>/snapshots/<sha[0..2]>/<sha256>
```

Identical files are stored once. The SQLite rows hold only metadata: the
snapshot id, the conversation, the step, and each file's path, hash and size.

## Staging and committing

Reverting is two steps on purpose.

**Stage** records the intent and changes nothing on disk. The user can see
which paths would be restored and then decide.

**Commit** restores the captured bytes. Each file is published through a
temporary file in the target's own directory followed by a rename, so a crash
cannot leave a half-written file behind. The report names:

| Bucket | Meaning |
| --- | --- |
| `restored` | Files written back to their captured content. |
| `deleted` | Files removed because the snapshot recorded them as absent. |
| `overwritten` | Files whose content had changed since the snapshot and were overwritten anyway. |
| `failed` | Files that could not be restored, with the reason. |

A commit with failures stays staged so it can be retried; a clean commit
clears the staged revert. Discarding a staged revert leaves the workspace
exactly as it was.

## Commands

| Command | Purpose |
| --- | --- |
| `list_snapshots` | The conversation's snapshots, newest first. |
| `stage_revert` | Stage a revert of one snapshot. |
| `discard_revert` | Forget a staged revert. |
| `commit_revert` | Restore the staged snapshot. |
| `capture_snapshot` | Capture an explicit set of paths. |

## Limits

Snapshots are pruned to the most recent few per conversation; pruning also
removes the blobs no surviving snapshot references. A snapshot is a
**workspace** record, not a conversation record: it restores files, it does
not rewind the conversation. To revisit an earlier conversation state, fork
the conversation.
