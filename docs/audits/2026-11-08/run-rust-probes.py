#!/usr/bin/env python3
"""Run diagnostic reproductions against a disposable copy of the working tree.

Passing probes confirm the documented bad behavior; they are not repair tests.
No application database, credential, real model, or browser is used.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
scratch = ROOT / ".local/qa/audit-2026-11-08"
scratch.mkdir(parents=True, exist_ok=True)
copy = Path(tempfile.mkdtemp(prefix="probe-workspace-", dir=scratch))
for name in ("src-tauri", "src", "catalog"):
    shutil.copytree(ROOT / name, copy / name, symlinks=True,
                    ignore=shutil.ignore_patterns("target", "node_modules", ".git"))
for name in ("package.json", "package-lock.json"):
    if (ROOT / name).exists():
        shutil.copy2(ROOT / name, copy / name)
for module in ("lib", "chat", "subagents"):
    with (copy / "src-tauri/src" / f"{module}.rs").open("a") as output:
        output.write("\n" + (HERE / f"rust-{module}-probes.inc").read_text())
print(f"Disposable source: {copy}", flush=True)
env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / "src-tauri/target"))
result = subprocess.run([
    "cargo", "test", "--manifest-path", str(copy / "src-tauri/Cargo.toml"),
    "--lib", "audit_20261108", "--", "--nocapture", "--test-threads=1",
], cwd=copy, env=env)
raise SystemExit(result.returncode)
