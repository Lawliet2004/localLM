#!/usr/bin/env python3
"""Run harness diagnostics in a disposable copy of the current working tree.

diag_* tests deliberately assert observed defects/limits. Passing diagnostics
are reproductions, not correctness gates. control_* tests assert protections.
No user database, credentials, live model, or browser is used.
"""
import json
import difflib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
scratch = ROOT / ".local/qa/harness-audit-2026-11-08"
scratch.mkdir(parents=True, exist_ok=True)
copy = Path(tempfile.mkdtemp(prefix="probe-workspace-", dir=scratch))
for name in ("src-tauri", "src", "catalog"):
    shutil.copytree(ROOT / name, copy / name, symlinks=True,
                    ignore=shutil.ignore_patterns("target", "node_modules", ".git"))
for name in ("package.json", "package-lock.json"):
    shutil.copy2(ROOT / name, copy / name)
for module in ("lib", "chat", "subagents"):
    with (copy / "src-tauri/src" / f"{module}.rs").open("a") as output:
        output.write("\n" + (HERE / f"{module}-probes.inc").read_text())
# Diagnostic control for the persistently failing ignored fixture. Keep its
# assertions intact and supply the extra response required by the new hook
# verification reminder. The original fixture's failures remain in separate logs.
chat_path = copy / "src-tauri/src/chat.rs"
original = chat_path.read_text()
start = original.index("    async fn permission_change_after_group_dispatch_preserves_both_real_results()")
end = original.index("    #[tokio::test]", start)
fragment = original[start:end]
before = 'for body in [wire, text_wire("Finished.", "stop")] {'
after = 'for body in [wire, text_wire("Finished.", "stop"), text_wire("Finished; no relevant source checks were performed.", "stop")] {'
assert fragment.count(before) == 1
fragment = fragment.replace(before, after)
adjusted = original[:start] + fragment + original[end:]
chat_path.write_text(adjusted)
(HERE / "integration-fixture-adjustment.diff").write_text("".join(difflib.unified_diff(
    original.splitlines(keepends=True), adjusted.splitlines(keepends=True),
    fromfile="original-chat.rs", tofile="disposable-chat.rs")))
(HERE / "probe-source-location.txt").write_text(str(copy) + "\n")
print(f"Disposable source: {copy}", flush=True)
env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / "src-tauri/target"))
command = ["cargo", "test", "--manifest-path", str(copy / "src-tauri/Cargo.toml"),
           "--lib", "audit_harness_20261108", "--", "--nocapture", "--test-threads=1"]
result = subprocess.run(command, cwd=copy, env=env, stdout=subprocess.PIPE,
                        stderr=subprocess.STDOUT, text=True)
print(result.stdout, end="", flush=True)
facts = []
for line in result.stdout.splitlines():
    if "AUDIT_EVIDENCE " in line:
        try:
            facts.append(json.loads(line.split("AUDIT_EVIDENCE ", 1)[1]))
        except json.JSONDecodeError:
            pass
(HERE / "probe-facts.json").write_text(json.dumps(facts, indent=2) + "\n")
control = subprocess.run(["cargo", "test", "--manifest-path", str(copy / "src-tauri/Cargo.toml"),
    "--lib", "chat::output_recovery_tests::permission_change_after_group_dispatch_preserves_both_real_results",
    "--", "--ignored", "--exact", "--nocapture", "--test-threads=1"], cwd=copy, env=env,
    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
(HERE / "loopback-fixture-control.txt").write_text(control.stdout)
print("Disposable three-response fixture control:", control.returncode, flush=True)
raise SystemExit(result.returncode or control.returncode)
