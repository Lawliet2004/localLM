#!/usr/bin/env python3
"""Build and freeze an immutable evaluator only after a successful build.

Input hashes are checked across compilation, so a concurrent source edit cannot
be attributed to the wrong binary. Never overwrites an earlier revision.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import time

ROOT = Path(__file__).resolve().parents[1]
QA = ROOT / '.local/qa/harness-simplification'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory():
    paths = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=ROOT).decode().split('\0')
    configs = {'package.json', 'package-lock.json', 'vite.config.ts', 'vitest.config.ts', 'tsconfig.json', 'tsconfig.node.json', 'tsconfig.app.json'}
    records = []
    for name in sorted(set(paths)):
        path = ROOT / name
        build_input = name.startswith(('src/', 'src-tauri/')) or name in configs or (name.startswith('scripts/') and (Path(name).name.startswith(('web-', 'build-web-', 'build-genui-'))))
        if build_input and path.is_file():
            records.append({'path': name, 'sha256': digest(path), 'bytes': path.stat().st_size})
    return records


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--revision', required=True)
    args = parser.parse_args()
    if not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,40}', args.revision):
        parser.error('revision must be a short lowercase identifier')
    destination = QA / f'candidate-{args.revision}'
    if destination.exists():
        parser.error('revision already exists; frozen builds are immutable')
    QA.mkdir(parents=True, exist_ok=True)
    log_path = QA / f'freeze-{args.revision}.log'
    started = time.monotonic()
    command = ['cargo', 'build', '--features', 'harness-eval', '--bin', 'locallm']
    with log_path.open('w') as log:
        subprocess.run(['npm', 'run', 'build:web'], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
        before = inventory()
        subprocess.run(command, cwd=ROOT / 'src-tauri', env={**os.environ, 'CARGO_BUILD_JOBS': '2'}, stdout=log, stderr=subprocess.STDOUT, check=True)
        if inventory() != before:
            raise RuntimeError('Build inputs changed during compilation; no candidate was frozen.')
    binary = ROOT / 'src-tauri/target/debug' / ('locallm.exe' if os.name == 'nt' else 'locallm')
    destination.mkdir()
    shutil.copy2(binary, destination / 'locallm')
    with tarfile.open(destination / 'source.tar.gz', 'w:gz') as archive:
        for record in before:
            archive.add(ROOT / record['path'], arcname=record['path'], recursive=False)
    if inventory() != before:
        raise RuntimeError('Build inputs changed while archiving. This revision is invalid; do not evaluate it.')
    manifest = {
        'gitHead': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT).decode().strip(),
        'binarySha256': digest(destination / 'locallm'),
        'sourceInventorySha256': hashlib.sha256(json.dumps(before, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
        'sourceArchiveSha256': digest(destination / 'source.tar.gz'),
        'inventoryMethod': 'Sorted nonignored source/config/resource and web-worker build input records, compact JSON with sorted keys; verified unchanged across build and archive.',
        'profile': 'debug (same as baseline and V7)', 'buildCommand': 'CARGO_BUILD_JOBS=2 ' + ' '.join(command),
        'buildLog': str(log_path.relative_to(QA)), 'buildSeconds': time.monotonic() - started,
        'files': before,
    }
    (destination / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps({key: value for key, value in manifest.items() if key != 'files'}))


if __name__ == '__main__':
    main()
