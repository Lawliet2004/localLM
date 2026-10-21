#!/usr/bin/env python3
"""Run installed development dependencies without installation or permission changes.
Strict acceptance requires every selected real acceptance test to execute and pass.
Only isolated disposable databases may be configured through fixture variables.
"""
import argparse
import datetime
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]


def executable(variable, fallback=None):
    value = os.environ.get(variable) or (shutil.which(fallback) if fallback else None)
    if not value:
        return None, 'Set ' + variable + ' to an installed absolute executable'
    path = pathlib.Path(value)
    if not path.is_absolute() or not path.is_file() or path.suffix.lower() in ('.cmd', '.bat'):
        return None, variable + ' must identify an actual absolute executable, not a shell shim'
    return str(path), None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--strict', action='store_true')
    parser.add_argument('--unit', action='store_true')
    parser.add_argument('--only', action='append', choices=['http', 'node', 'debugpy', 'codelldb', 'rust-lsp', 'typescript-lsp', 'postgres', 'mysql', 'native-profiler'])
    parser.add_argument('--output', type=pathlib.Path, default=pathlib.Path(tempfile.gettempdir()) / 'locallm-development-acceptance')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    records = []
    cargo = shutil.which('cargo') or 'cargo'
    node = shutil.which('node') or 'node'

    def run(name, command, cwd, missing=None, required=True, real=False):
        log = args.output / (name + '.log')
        if missing:
            log.write_text(missing + '\n', encoding='utf-8')
            record = dict(name=name, required=required, status='blocked', reason=missing, log=str(log))
        else:
            started = datetime.datetime.now(datetime.timezone.utc)
            try:
                with log.open('w', encoding='utf-8') as out:
                    code = subprocess.run(command, cwd=cwd, stdout=out, stderr=subprocess.STDOUT, timeout=900).returncode
                output = log.read_text(encoding='utf-8', errors='replace')
                status = 'passed' if code == 0 else 'failed'
                # cargo returns zero for an absent filter. That is missing
                # acceptance, never proof that the dependency works.
                matches = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;', output)
                if real and code == 0 and (not matches or sum(int(m[0]) for m in matches) < {'http': 2, 'rust-lsp': 2, 'typescript-lsp': 2}.get(name, 1)):
                    status = 'not-run'
                record = dict(name=name, required=required, status=status, exitCode=code, log=str(log), elapsedSeconds=(datetime.datetime.now(datetime.timezone.utc)-started).total_seconds())
            except (OSError, subprocess.TimeoutExpired) as error:
                with log.open('a', encoding='utf-8') as out:
                    out.write(str(error))
                record = dict(name=name, required=required, status='failed', reason=str(error), log=str(log))
        records.append(record)
        print(name + ': ' + record['status'].upper() + (' — ' + record['reason'] if 'reason' in record else ''), flush=True)

    units = [
        ('rust-unit', [cargo, 'test', '--lib'], ROOT/'src-tauri'),
        ('rust-check', [cargo, 'check', '--lib'], ROOT/'src-tauri'),
        ('typecheck', [node, str(ROOT/'node_modules/typescript/bin/tsc'), '--noEmit'], ROOT),
        ('frontend', [node, str(ROOT/'node_modules/vitest/vitest.mjs'), 'run'], ROOT),
    ]
    for name, command, cwd in units:
        if args.unit:
            run(name, command, cwd)
        else:
            records.append(dict(name=name, required=False, status='not-run', reason='Enable --unit'))

    cc, cc_missing = executable('LOCALLM_CC', 'cc')
    if cc:
        os.environ['LOCALLM_CC'] = cc
    tests = [
        ('http', 'api_tools::transport_tests::actual_http_', None),
        ('node', 'real_node_inspector_capture', None if shutil.which('node') else 'Installed Node is unavailable'),
    ]
    for name, variable, version, test in [
        ('debugpy', 'LOCALLM_DEBUGPY_PYTHON', ['-c', 'import debugpy'], 'debug_tools::acceptance::real_debugpy'),
        ('rust-lsp', 'LOCALLM_RUST_ANALYZER', ['--version'], 'real_rust_'),
        ('typescript-lsp', 'LOCALLM_TYPESCRIPT_LANGUAGE_SERVER', None, 'real_typescript_'),
    ]:
        path, missing = executable(variable)
        if not missing and version:
            try:
                probe = subprocess.run([path] + version, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=15)
                if probe.returncode:
                    missing = variable + ' prerequisite probe failed (adapter module/component unavailable)'
            except (OSError, subprocess.TimeoutExpired):
                missing = variable + ' prerequisite probe failed'
        tests.append((name, test, missing))
    _, missing = executable('LOCALLM_CODELLDB')
    tests.append(('codelldb', 'debug_tools::acceptance::real_codelldb', missing or cc_missing))
    for engine, label in [('PG', 'postgres'), ('MYSQL', 'mysql')]:
        missing = None
        for suffix in ['PORT', 'PASSWORD', 'DUMP', 'RESTORE']:
            variable = 'LOCALLM_ACCEPTANCE_' + engine + '_' + suffix
            if not os.environ.get(variable):
                missing = 'Provide isolated disposable ' + label + ' fixture variables; see docs/DEVELOPMENT_COMPLETION.md'
            elif suffix in ('DUMP', 'RESTORE'):
                _, invalid = executable(variable)
                missing = missing or invalid
        tests.append((label, 'real_' + label + '_readonly_and_disposable_rehearsal', missing))
    profiler, missing = executable('LOCALLM_NATIVE_PROFILER', 'perf' if sys.platform.startswith('linux') else None)
    if not missing and (not args.only or 'native-profiler' in args.only):
        os.environ['LOCALLM_NATIVE_PROFILER'] = profiler
        if sys.platform.startswith('linux'):
            with tempfile.TemporaryDirectory(prefix='locallm-perf-probe-') as temp:
                try:
                    probe = subprocess.run([profiler, 'record', '-e', 'cpu-clock:u', '--call-graph', 'dwarf', '-o', str(pathlib.Path(temp)/'probe.data'), '--', shutil.which('true') or '/usr/bin/true'], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=15)
                    (args.output/'native-profiler-probe.log').write_bytes(probe.stdout)
                    if probe.returncode:
                        reason = probe.stdout.decode(errors='replace')
                        missing = ('Native perf recording is permission-blocked' if any(s in reason.lower() for s in ['permission', 'operation not permitted', 'perf_event_paranoid', 'access to performance monitoring']) else 'Native perf recording prerequisite failed') + '; see native-profiler-probe.log. No OS permissions changed.'
                except (OSError, subprocess.TimeoutExpired):
                    missing = 'Native perf probe could not complete; no OS permissions changed'
        elif sys.platform == 'win32' and not all(os.environ.get(v) for v in ['LOCALLM_PROFILE_EXPORTER', 'LOCALLM_WPA_PROFILE']):
            missing = 'Configure installed WPA exporter and a CPU export profile'
    tests.append(('native-profiler', 'real_native_profiler_capture', missing or cc_missing))

    selected = set(args.only or [name for name, _, _ in tests])
    for name, test, missing in tests:
        if name not in selected:
            records.append(dict(name=name, required=False, status='skipped', reason='Not selected by --only'))
            continue
        run(name, [cargo, 'test', '--lib', test, '--', '--ignored', '--nocapture'], ROOT/'src-tauri', missing, real=True)
    report = dict(schemaVersion='locallm.development-acceptance/1', createdAt=datetime.datetime.now(datetime.timezone.utc).isoformat(), platform=sys.platform, strict=args.strict, results=records)
    report['accepted'] = all(r['status'] == 'passed' for r in records if r['required'])
    (args.output/'report.json').write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
    return 1 if any(r['status'] == 'failed' or args.strict and r['required'] and r['status'] != 'passed' for r in records) else 0


if __name__ == '__main__':
    raise SystemExit(main())
