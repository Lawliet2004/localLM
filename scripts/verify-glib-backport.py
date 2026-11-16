#!/usr/bin/env python3
"""Check the GLib compatibility patch against the original crate's file hashes."""
import hashlib
import json
from pathlib import Path
import tomllib

root = Path(__file__).resolve().parents[1] / 'src-tauri/vendor/glib'
upstream = json.loads((root / 'UPSTREAM-FILES.json').read_text())
patch = json.loads((root / 'LOCALLM-BACKPORT.json').read_text())
assert upstream['package'] == patch['cratesIoChecksum'], 'Upstream package identity changed'
assert tomllib.loads((root / 'Cargo.toml').read_text())['package']['version'] == '0.18.5', 'Do not disguise the backport as a later release'
changed = patch['changedSourceFile']
assert upstream['files'][changed] == patch['originalSourceSha256']
for name, expected in upstream['files'].items():
    if name == changed:
        expected = patch['patchedSourceSha256']
    actual = hashlib.sha256((root / name).read_bytes()).hexdigest()
    assert actual == expected, f'Unexpected vendored source change: {name}'
source = (root / changed).read_text()
assert 'let mut p: *mut libc::c_char = std::ptr::null_mut();' in source
assert '                &mut p,' in source
allowed = {'Cargo.lock', 'LOCALLM-BACKPORT.md', 'LOCALLM-BACKPORT.json',
           'UPSTREAM-FILES.json', 'tests/variant_iter_backport.rs'}
extra = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()} - set(upstream['files']) - allowed
assert not extra, f'Unreviewed files in vendored dependency: {sorted(extra)}'
print(f'GLib backport verified: {len(upstream["files"])} upstream file hashes; one patched source file; version unchanged.')
