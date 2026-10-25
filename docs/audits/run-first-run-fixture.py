"""Run the audit fixture without changing application code or user data.

Run `cargo test --lib --no-run` in src-tauri first to build dependencies.
The fixture is a snapshot, not a regression test against future source edits.
"""
from pathlib import Path
import subprocess
import tempfile

repo = Path(__file__).resolve().parents[2]
deps = repo / "src-tauri/target/debug/deps"
fixture = Path(__file__).with_name("first-run-2026-10-22-fixture.rs")
with tempfile.TemporaryDirectory(prefix="locallm-audit-review-") as scratch:
    output = Path(scratch) / "fixture"
    args = ["rustc", "--edition=2021", "--crate-name", "first_run_audit",
            str(fixture), "-o", str(output), "-L", f"dependency={deps}"]
    for crate in ["serde", "serde_json", "cap_std"]:
        libraries = sorted(deps.glob(f"lib{crate}-*.rlib"),
                           key=lambda item: item.stat().st_mtime, reverse=True)
        if not libraries:
            raise SystemExit(f"Build backend test dependencies first: missing {crate}")
        args += ["--extern", f"{crate}={libraries[0]}"]
    subprocess.run(args, check=True)
    subprocess.run([str(output), str(Path(scratch) / "data")], check=True)
