# Backend audit reproductions

These are **diagnostic reproductions of remaining defects**, not proposed fixes. The tests intentionally assert the defective behavior. A passing reproduction means the problem was observed. Production regression tests should assert the repaired behavior instead.

The audit ran against an isolated source copy with temporary SQLite databases and workspaces. It did not mutate the application's database or the repository's production source files. The snapshot test demonstrates an outside-workspace overwrite using two disposable folders, never a real external file.

- [Exact final probe output](/home/papan/Desktop/Projects/localLM/docs/backend-review-2026-11-06/probe-results.txt): 14 diagnostic checks reproduced their conditions.
- [Reproduction patch](/home/papan/Desktop/Projects/localLM/docs/backend-review-2026-11-06/repro.patch): appends test modules to six Rust source files; no production implementation is replaced.
- [Inventory](/home/papan/Desktop/Projects/localLM/docs/backend-review-2026-11-06/source-inventory.csv): paths, line counts including tests, and SHA-256 hashes of the reviewed original source.
- [Findings and repair order](/home/papan/Desktop/Projects/localLM/docs/BACKEND_REVIEW_2026-11-06.md).

Apply the patch only to a **disposable copy of the reviewed working tree**, including its existing uncommitted files. A fresh checkout of HEAD is not equivalent to that tree. Compare the inventory hashes if the source has changed since the review.

From the root of that disposable copy:

```bash
git apply /home/papan/Desktop/Projects/localLM/docs/backend-review-2026-11-06/repro.patch
cargo test --manifest-path src-tauri/Cargo.toml --lib audit_review_ -- --nocapture
```

The original run reused the repository's Cargo target directory for dependency compilation. The patch itself changes no original source when applied to the disposable copy. Unix-only process, shell, and symlink probes are platform-gated; other platforms will run fewer than 14 probes. The shell-output fixture uses the host's standard `head` and `tr` commands.

The audit also explicitly ran 19 normally ignored local integration tests, all passing, with the harness filters `--ignored chat:: api_tools:: subagents::`. Those tests use scripted local servers and temporary files, launch no browser, and make no paid model request. Their passing result validates those covered paths, not the defective cases documented here.
