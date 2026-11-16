#!/usr/bin/env python3
"""Run real database acceptance against owned, disposable Docker fixtures.

Never uses saved application profiles, existing containers or host databases.
Native dump/restore clients run inside each fixture through explicit launchers.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--test-binary', type=Path, required=True)
    parser.add_argument('--engine', choices=['postgres', 'mysql', 'all'], default='all')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--postgres-image', default='postgres:17')
    parser.add_argument('--mysql-image', default='mysql:8.4')
    parser.add_argument('--skip-pull', action='store_true', help='Use already verified local image(s); record their digests')
    args = parser.parse_args()
    binary = args.test_binary.resolve(strict=True)
    docker = shutil.which('docker')
    if not docker:
        parser.error('Docker is required; no database fixture was started')
    host = os.environ.get('DOCKER_HOST') or json.loads(subprocess.check_output(
        [docker, 'context', 'inspect', '--format', '{{json .Endpoints.docker.Host}}'], text=True))
    if not host.startswith('unix://'):
        parser.error('Acceptance fixtures require a local Unix Docker socket')
    docker_command = [docker, '--host', host]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report_path = output / 'report.json'
    if report_path.exists():
        parser.error('Choose a new output directory; an earlier report exists')
    results = []
    for engine in (['postgres', 'mysql'] if args.engine == 'all' else [args.engine]):
        started = time.monotonic()
        name = 'locallm-acceptance-' + uuid.uuid4().hex
        directory = output / engine
        directory.mkdir()
        secret = secrets.token_urlsafe(32)
        image = args.postgres_image if engine == 'postgres' else args.mysql_image
        port = 5432 if engine == 'postgres' else 3306
        variable = 'POSTGRES_PASSWORD' if engine == 'postgres' else 'MYSQL_ROOT_PASSWORD'
        env = dict(os.environ, **{variable: secret})
        result = dict(engine=engine, imageTag=image, container=name, status='failed', cleaned=False)
        try:
            if not args.skip_pull:
                with (directory / 'pull.log').open('w') as log:
                    subprocess.run(docker_command + ['pull', image], stdout=log, stderr=log, check=True, timeout=600)
            result['imageDigests'] = json.loads(subprocess.check_output(
                docker_command + ['image', 'inspect', image, '--format', '{{json .RepoDigests}}'], text=True))
            command = docker_command + ['run', '--detach', '--rm', '--name', name,
                       '--memory', '768m', '--cpus', '1.0', '--pids-limit', '128',
                       '--publish', f'127.0.0.1::{port}', '--env', variable]
            if engine == 'postgres':
                command += ['--env', 'POSTGRES_DB=locallm_acceptance', image]
            else:
                command += ['--env', 'MYSQL_DATABASE=locallm_acceptance', '--env', 'MYSQL_ROOT_HOST=%', image,
                            '--performance-schema=OFF', '--innodb-buffer-pool-size=64M',
                            '--innodb-redo-log-capacity=64M', '--max-connections=20']
            subprocess.run(command, env=env, stdout=subprocess.DEVNULL, check=True, timeout=120)
            deadline = time.monotonic() + 180
            while True:
                probe = docker_command + (['exec', name, 'pg_isready', '-h', '127.0.0.1', '-U', 'postgres'] if engine == 'postgres' else [
                    'exec', name, 'mysqladmin', '--protocol=TCP', '--host=127.0.0.1', 'ping', '--silent'])
                check = subprocess.run(probe, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10)
                if check.returncode == 0:
                    break
                if time.monotonic() > deadline:
                    raise TimeoutError('Owned database fixture did not become ready')
                time.sleep(.5)
            endpoint = subprocess.check_output(docker_command + ['port', name, f'{port}/tcp'], text=True).strip()
            if not endpoint.startswith('127.0.0.1:') or '\n' in endpoint:
                raise ValueError('Fixture must publish one loopback-only endpoint')
            external_port = endpoint.rsplit(':', 1)[1]
            result['hostPort'] = int(external_port)
            if engine == 'mysql':
                # A configured mysql client may read options from its home.
                # This deliberately incompatible option proves the rehearsal
                # launches with the selected profile's options only.
                subprocess.run(docker_command + ['exec', '-i', name, 'sh', '-c',
                    'cat > /root/.my.cnf'], input='[client]\nlocallm-acceptance-invalid-option\n',
                    text=True, check=True, timeout=15)
                control = subprocess.run(docker_command + ['exec', '--env', 'HOME=/root',
                    name, 'mysql', '--print-defaults'], capture_output=True, text=True, timeout=15)
                if control.returncode != 0 or '--locallm-acceptance-invalid-option' not in control.stdout:
                    raise ValueError('MySQL client did not observe the fixture option file')
                result['isolatedClientOptionsControl'] = True
            paths = {}
            for role, program in [('dump', 'pg_dump' if engine == 'postgres' else 'mysqldump'),
                                  ('restore', 'psql' if engine == 'postgres' else 'mysql')]:
                launcher = directory / role
                # Arguments and credentials remain data. No shell command is built.
                source = f'''#!{sys.executable}
import os, subprocess, sys
args = sys.argv[1:]
for index, value in enumerate(args):
    if value == '--port': args[index + 1] = {str(port)!r}
    elif value.startswith('--port='): args[index] = '--port={port}'
command = {docker_command!r} + ['exec', '-i']
if {engine == 'mysql'!r}: command += ['--env', 'HOME=/root']
for key in ['PGPASSWORD', 'PGSSLMODE', 'MYSQL_PWD', 'MYSQL_TEST_LOGIN_FILE']:
    if key in os.environ: command += ['--env', key]
command += [{name!r}, {program!r}] + args
completed = subprocess.run(command, stderr=subprocess.PIPE)
diagnostic = completed.stderr.decode('utf-8', errors='replace')
for key in ['PGPASSWORD', 'MYSQL_PWD']:
    if os.environ.get(key): diagnostic = diagnostic.replace(os.environ[key], '[redacted]')
sys.stderr.write(diagnostic)
with open({str(directory / (role + '-diagnostics.log'))!r}, 'a') as log:
    log.write('exit=' + str(completed.returncode) + '\\n' + diagnostic[:65536] + '\\n')
sys.exit(completed.returncode)
'''
                launcher.write_text(source)
                launcher.chmod(0o700)
                paths[role] = str(launcher)
            prefix = 'LOCALLM_ACCEPTANCE_' + ('PG' if engine == 'postgres' else 'MYSQL')
            test_env = dict(os.environ, **{prefix + '_PORT': external_port,
                prefix + '_PASSWORD': secret, prefix + '_DUMP': paths['dump'], prefix + '_RESTORE': paths['restore']})
            label = 'real_' + engine + '_readonly_and_disposable_rehearsal'
            with (directory / 'acceptance.log').open('w') as log:
                code = subprocess.run([str(binary), '--exact', 'remote_database::acceptance::' + label,
                    '--ignored', '--nocapture', '--test-threads=1'], cwd=ROOT/'src-tauri', env=test_env,
                    stdout=log, stderr=log, timeout=600).returncode
            test_log = directory / 'acceptance.log'
            text = test_log.read_text().replace(secret, '[redacted]')
            test_log.write_text(text)
            result['exitCode'] = code
            result['status'] = 'passed' if code == 0 and '1 passed; 0 failed' in text else 'failed'
        except (OSError, subprocess.SubprocessError, ValueError) as error:
            result['error'] = str(error).replace(secret, '[redacted]')
        finally:
            try:
                logs = subprocess.run(docker_command + ['logs', name], capture_output=True, text=True, timeout=15)
                (directory / 'container.log').write_text((logs.stdout + logs.stderr).replace(secret, '[redacted]'))
            except (OSError, subprocess.SubprocessError) as error:
                result['containerLogError'] = str(error).replace(secret, '[redacted]')
            try:
                subprocess.run(docker_command + ['rm', '--force', name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
                remaining = subprocess.run(docker_command + [
                    'ps', '--all', '--filter', f'name=^/{name}$', '--format', '{{.Names}}',
                ], capture_output=True, text=True, check=True, timeout=15)
                result['cleaned'] = not remaining.stdout.strip()
                if not result['cleaned']:
                    result['cleanupError'] = 'Owned fixture is still present.'
            except (OSError, subprocess.SubprocessError) as error:
                result['cleanupError'] = str(error).replace(secret, '[redacted]')
            result['elapsedSeconds'] = time.monotonic() - started
            results.append(result)
            report_path.write_text(json.dumps({'schemaVersion': 'locallm.database-acceptance/1',
                'recordedAt': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'results': results}, indent=2) + '\n')
        print(engine + ': ' + result['status'] + ', fixture removed=' + str(result['cleaned']), flush=True)
    return 0 if all(row['status'] == 'passed' and row['cleaned'] for row in results) else 1


if __name__ == '__main__':
    raise SystemExit(main())
