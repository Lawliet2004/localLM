#!/usr/bin/env python3
"""Compare isolated LocalLM and Claude Code against one loopback model.

This is a small compatibility/efficiency experiment, not a Claude model eval.
No account credentials, saved settings, browser or network tools are used.
"""
import argparse
from contextlib import closing
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import random
import re
import shutil
import signal
import stat
import subprocess
import sys
import threading
import tempfile
import time
import urllib.parse
import urllib.request

MODEL = 'locallm-benchmark'
MAX_OUTPUT = 1024
MAX_TURNS = 64
TIMEOUT_SECONDS = 240
CLAUDE_TOOLS = ['Read', 'Glob', 'Grep', 'Write', 'Edit']
GENERATION_ROUTES = {'/v1/chat/completions', '/v1/messages'}
TOKEN_COUNT_ROUTE = '/v1/messages/count_tokens'


def events(body):
    body = body.decode('utf-8-sig')
    try:
        return [json.loads(body)]
    except ValueError:
        frames = []
        pending = []
        for line in re.split(r'\r\n|\r|\n', body):
            if line.startswith('data:'):
                pending.append(line[5:].removeprefix(' '))
            elif not line and pending:
                data = '\n'.join(pending)
                if data != '[DONE]':
                    frames.append(json.loads(data))
                pending.clear()
        if pending or not frames:
            raise ValueError('Missing or unterminated model response events')
        return frames


def measure(frames, anthropic):
    counts = {}
    calls = set()
    for event in frames:
        current = event.get('usage') or event.get('message', {}).get('usage') or {}
        counts.update(current)
        timings = event.get('timings') or {}
        if 'cache_n' in timings:
            counts.setdefault('cached', timings['cache_n'])
        if anthropic:
            if event.get('type') == 'content_block_start' and event['content_block']['type'] == 'tool_use':
                calls.add(event['index'])
        else:
            for choice in event.get('choices', []):
                for call in (choice.get('delta') or {}).get('tool_calls') or []:
                    calls.add((choice.get('index', 0), call['index']))
    if anthropic:
        cached = counts.get('cache_read_input_tokens')
        uncached = counts.get('input_tokens')
        if uncached is not None:
            uncached += counts.get('cache_creation_input_tokens', 0)
        total = uncached + cached if uncached is not None and cached is not None else None
    else:
        cached = (counts.get('prompt_tokens_details') or {}).get('cached_tokens', counts.get('cached'))
        total = counts.get('prompt_tokens')
        uncached = total - cached if total is not None and cached is not None and 0 <= cached <= total else None
    return dict(inputTokens=total, cachedTokens=cached, uncachedInputTokens=uncached,
                modelIssuedToolCalls=len(calls),
                outputTokens=counts.get('output_tokens', counts.get('completion_tokens')))


def grade(answer, expected):
    def unique_object(pairs):
        value = dict(pairs)
        if len(value) != len(pairs):
            raise ValueError('duplicate JSON key')
        return value

    text = answer.strip()
    fenced = re.fullmatch(r'```(?:json)?[ \t]*\r?\n(.*?)\r?\n```', text, re.DOTALL)
    try:
        actual = json.loads(fenced[1] if fenced else text, object_pairs_hook=unique_object)
    except ValueError:
        actual = None
    if not isinstance(actual, dict):
        return dict(formatCorrect=False, factsCorrect=False)
    typed = all(key in actual and type(actual[key]) is type(value) for key, value in expected.items())
    return dict(formatCorrect=fenced is None and typed and actual.keys() == expected.keys(),
                factsCorrect=typed and all(actual[key] == value for key, value in expected.items()))


def verify_grader():
    cases = [
        ('{"n": 1}', True, True),
        ('{"n": 2}', True, False),
        ('{"n": 1, "extra": 2}', False, True),
        ('```json\n{"n": 1}\n```', False, True),
        ('{"n": true}', False, False),
        ('{"n": 1.0}', False, False),
        ('{"n": 0, "n": 1}', False, False),
        ('Answer: {"n": 1}', False, False),
        ('{"n": 1} {"n": 1}', False, False),
        ('["n", 1]', False, False),
    ]
    for answer, correct_format, correct_facts in cases:
        if grade(answer, {'n': 1}) != dict(formatCorrect=correct_format, factsCorrect=correct_facts):
            raise AssertionError(f'Grader misclassified {answer!r}')
    return len(cases)


def snapshot(workspace):
    # Stat and hash all original top-level fixture files. The directory stamp
    # catches transient additions, and file ctime catches a write-and-restore.
    def stamp(info):
        return (info.st_mode, info.st_ino, info.st_ctime_ns, info.st_mtime_ns, info.st_size)
    root = stamp(workspace.stat())
    entries = {}
    for item in workspace.iterdir():
        info = item.lstat()
        digest = hashlib.sha256(item.read_bytes()).hexdigest() if stat.S_ISREG(info.st_mode) else None
        entries[item.name] = (stamp(info), digest)
    return root, entries


def run_child(command, workspace, env, stdout, stderr):
    with subprocess.Popen(command, cwd=workspace, env=env, stdout=stdout, stderr=stderr,
                          stdin=subprocess.DEVNULL, start_new_session=True) as child:
        try:
            return child.wait(timeout=TIMEOUT_SECONDS)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
            raise


def read_claude_run(path, *, timed_out=False):
    emitted = []
    with path.open('rb') as source:
        for line in source:
            try:
                emitted.append(json.loads(line))
            except ValueError:
                if timed_out and not line.endswith(b'\n'):
                    break
                raise
    result = next((event for event in reversed(emitted) if event['type'] == 'result'), None)
    if result is None and not timed_out:
        raise ValueError('Claude Code emitted no terminal result')
    return emitted, result


def tool_operations(harness, result, emitted):
    if harness == 'LocalLM':
        blocked = {'authorization-changed', 'capability-disabled', 'denied',
                   'hook-blocked', 'tool-budget-blocked', 'unavailable-tool'}
        audits = result['toolAudits']
        settled = [audit for audit in audits if audit['status'] == 'complete'
                   and audit['request'].get('decision') == 'allowed' and audit['errorCode'] not in blocked]
        return dict(recognizedToolCalls=result['recognizedCalls'], underlyingToolOperations=len(settled),
                    successfulToolOperations=sum(not audit['failed'] for audit in settled),
                    permissionDenials=[audit['request']['name'] for audit in audits
                                       if audit['request'].get('decision') == 'denied'])
    # Claude emits the native result as well as tool_use_result for executed
    # built-ins. Denied calls may have a model-visible error result too.
    denials = result.get('permission_denials', []) if result is not None else None
    denied = {item.get('tool_use_id') for item in denials or []}
    results = {}
    for event in emitted:
        if event['type'] != 'user':
            continue
        for block in event.get('message', {}).get('content', []):
            if (isinstance(block, dict) and block.get('type') == 'tool_result'
                    and block['tool_use_id'] not in denied and 'tool_use_result' in event):
                results[block['tool_use_id']] = not block.get('is_error', False)
    recognized = {block['id'] for event in emitted if event['type'] == 'assistant'
                  for block in event['message']['content'] if block['type'] == 'tool_use'}
    return dict(recognizedToolCalls=len(recognized), underlyingToolOperations=len(results),
                successfulToolOperations=sum(results.values()), permissionDenials=denials,
                toolOperationCountsPartial=result is None)


def verify_timeout_trace():
    trace = [
        dict(type='assistant', message={'content': [
            {'type': 'tool_use', 'id': 'settled'}, {'type': 'tool_use', 'id': 'pending'}]}),
        dict(type='user', message={'content': [
            {'type': 'tool_result', 'tool_use_id': 'settled'}]}, tool_use_result={}),
    ]
    with tempfile.TemporaryDirectory() as temp:
        path = Path(temp) / 'stdout.log'
        path.write_text(''.join(json.dumps(event) + '\n' for event in trace) + '{"type":"result"')
        emitted, result = read_claude_run(path, timed_out=True)
        counts = tool_operations('Claude Code', result, emitted)
        assert counts == dict(recognizedToolCalls=2, underlyingToolOperations=1,
                              successfulToolOperations=1, permissionDenials=None,
                              toolOperationCountsPartial=True)
        try:
            read_claude_run(path)
        except ValueError:
            pass
        else:
            raise AssertionError('Accepted a truncated Claude Code trace after a normal exit')


def main():
    if sys.argv[1:] == ['--self-test']:
        print(f'{verify_grader()} grader cases passed')
        verify_timeout_trace()
        print('Timed-out tool trace case passed')
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base-url', required=True)
    parser.add_argument('--test-binary', type=Path, required=True)
    parser.add_argument('--claude', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--repetitions', type=int, default=3, choices=range(1, 6))
    args = parser.parse_args()
    backend = urllib.parse.urlsplit(args.base_url)
    if (backend.scheme != 'http' or backend.hostname != '127.0.0.1' or backend.port is None
            or backend.username or backend.password or backend.path or backend.query or backend.fragment):
        parser.error('Use an owned HTTP loopback model server')
    strace = shutil.which('strace')
    if not strace:
        parser.error('strace is required to record connection attempts during the comparison')
    grader_cases = verify_grader()
    binary, claude = args.test_binary.resolve(strict=True), args.claude.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    active = None
    records = []
    pending = 0
    condition = threading.Condition()

    class Proxy(BaseHTTPRequestHandler):
        def log_message(self, *arguments):
            pass

        def do_POST(self):
            nonlocal pending
            route = urllib.parse.urlsplit(self.path).path
            started = time.monotonic()
            with condition:
                if active is None:
                    self.send_error(503, 'No active comparison run')
                    return
                record = dict(run=active, path=self.path, generation=route in GENERATION_ROUTES)
                records.append(record)
                pending += 1
            try:
                length = int(self.headers.get('Content-Length', 0))
                if not 0 < length <= 4 * 1024 * 1024:
                    raise ValueError('Request exceeds fixture bound or is empty')
                request_body = self.rfile.read(length)
                request = json.loads(request_body)
                record['request'] = request
                if route not in GENERATION_ROUTES | {TOKEN_COUNT_ROUTE}:
                    raise ValueError(f'Unexpected inference route {route}')
                if record['generation'] and (
                    request.get('model') != MODEL or request.get('max_tokens') != MAX_OUTPUT
                    or request.get('temperature', 0) != 0 or request.get('top_p', 1) != 1
                ):
                    raise ValueError('Harness requested a different model or generation limit')
                record['forwarded'] = True
                with closing(http.client.HTTPConnection(backend.hostname, backend.port, timeout=180)) as connection:
                    connection.request('POST', self.path, body=request_body, headers={
                        'Content-Type': 'application/json', 'anthropic-version': '2023-06-01',
                        'x-api-key': 'locallm-fixture-only'})
                    response = connection.getresponse()
                    body = response.read(32 * 1024 * 1024 + 1)
                if len(body) > 32 * 1024 * 1024:
                    raise ValueError('Response exceeds fixture bound')
                record.update(status=response.status, response=body.decode(errors='replace'),
                              elapsedMs=(time.monotonic()-started)*1000)
                if record['generation'] and 200 <= response.status < 300:
                    try:
                        record['usage'] = measure(events(body), route == '/v1/messages')
                    except (ValueError, TypeError, KeyError, AttributeError) as error:
                        record['measurementError'] = str(error)
                self.send_response(response.status)
                self.send_header('Content-Type', response.getheader('Content-Type', 'application/json'))
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            except (OSError, ValueError, http.client.HTTPException) as error:
                record['error'] = str(error)
                if 'status' not in record:
                    self.send_error(502, 'Owned fixture proxy failed')
            finally:
                with condition:
                    pending -= 1
                    condition.notify_all()

    proxy = ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
    proxy.daemon_threads = True
    threading.Thread(target=proxy.serve_forever, daemon=True).start()
    base = f'http://127.0.0.1:{proxy.server_port}'
    results = []
    # A workspace under this repository inherits its AGENTS.md in LocalLM.
    # Keep fixture inputs outside it so both harnesses see only the task files.
    fixtures = tempfile.TemporaryDirectory(prefix='locallm-harness-comparison-', dir='/tmp')
    try:
        for repetition in range(args.repetitions):
            for task_index, task in enumerate(['single_read', 'four_file_join']):
                fixture = output / f'{repetition}-{task}'
                fixture.mkdir(parents=True)
                workspace = Path(fixtures.name) / f'{repetition}-{task}'
                workspace.mkdir(parents=True)
                rng = random.Random(20261108 + repetition)
                if task == 'single_read':
                    expected = {'code': hashlib.sha256(str(rng.random()).encode()).hexdigest()[:12]}
                    (workspace/'answer.txt').write_text('Verification code: ' + expected['code'] + '\n')
                    prompt = 'Read answer.txt. Return exactly one JSON object with the key code and its verification code. Do not modify files or run commands.'
                else:
                    expected = {key: rng.randrange(17, 200) for key in ['alpha', 'beta', 'gamma', 'delta']}
                    for key, value in expected.items():
                        (workspace/f'{key}.json').write_text(json.dumps({'value': value})+'\n')
                    expected['total'] = sum(expected.values())
                    prompt = 'Read alpha.json, beta.json, gamma.json, and delta.json. Return exactly one JSON object with keys alpha, beta, gamma, delta containing each file value, plus total containing their sum. Do not modify files or run commands.'
                fixture_hashes = {f.name: hashlib.sha256(f.read_bytes()).hexdigest() for f in workspace.iterdir()}
                shutil.copytree(workspace, fixture/'inputs')
                (fixture/'task.json').write_text(json.dumps(dict(prompt=prompt, expected=expected, inputHashes=fixture_hashes), indent=2))
                # Alternate which harness runs first; erase the same slot before
                # every cold run. Warm repeats immediately reuse that harness.
                order = ['LocalLM', 'Claude Code'] if (repetition + task_index) % 2 == 0 else ['Claude Code', 'LocalLM']
                for harness in order:
                    erase = urllib.request.Request(args.base_url+'/slots/0?action=erase', b'', method='POST')
                    with urllib.request.urlopen(erase, timeout=10) as response:
                        erased = json.load(response)
                    for temperature in ['cold', 'warm']:
                        # Keep every run's input identical even if a model
                        # violates the read-only task. That violation is graded
                        # before this restoration, and its output is retained.
                        for entry in workspace.iterdir():
                            if entry.is_symlink() or entry.is_file(): entry.unlink()
                            else: shutil.rmtree(entry)
                        for entry in (fixture/'inputs').iterdir(): shutil.copyfile(entry, workspace/entry.name)
                        initial = snapshot(workspace)
                        assert {name: entry[1] for name, entry in initial[1].items()} == fixture_hashes
                        name = f'{repetition}-{task}-{harness.replace(" ", "-")}-{temperature}'
                        directory = fixture / f'{harness.replace(" ", "-")}-{temperature}'
                        directory.mkdir()
                        with condition:
                            active = name
                        started = time.monotonic()
                        row = dict(run=name, harness=harness, task=task, repetition=repetition,
                                   cacheState=temperature, slotErase=erased if temperature == 'cold' else None,
                                   success=False, formatCorrect=False, factsCorrect=False)
                        # Auth, user config, plugin settings and proxy variables are
                        # absent. A fresh home is independent for every run.
                        home = directory/'home'
                        home.mkdir(mode=0o700)
                        env = {key: os.environ[key] for key in ('PATH', 'LANG', 'LC_ALL', 'TZ') if key in os.environ}
                        env.update(HOME=str(home), XDG_CONFIG_HOME=str(home/'config'),
                                   XDG_DATA_HOME=str(home/'data'), XDG_CACHE_HOME=str(home/'cache'))
                        if harness == 'LocalLM':
                            config = dict(workspace=str(workspace), baseUrl=base+'/v1', modelId=MODEL,
                                          prompt=prompt, output=str(directory/'result.json'))
                            config_path = directory/'config.json'
                            config_path.write_text(json.dumps(config, indent=2))
                            env['LOCALLM_COMPARISON_CONFIG'] = str(config_path)
                            command = [str(binary), '--exact', 'harness_comparison_tests::registered_local_model_comparison', '--ignored', '--nocapture', '--test-threads=1']
                        else:
                            config_dir = home/'claude'
                            config_dir.mkdir()
                            env.update(CLAUDE_CONFIG_DIR=str(config_dir), ANTHROPIC_BASE_URL=base,
                                       ANTHROPIC_API_KEY='locallm-fixture-only', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1',
                                       DISABLE_TELEMETRY='1', DISABLE_ERROR_REPORTING='1',
                                       CLAUDE_CODE_MAX_OUTPUT_TOKENS=str(MAX_OUTPUT), MAX_THINKING_TOKENS='0')
                            command = [str(claude), '--bare', '--print', '--model', MODEL,
                                       '--output-format', 'stream-json', '--verbose', '--no-session-persistence',
                                       '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}',
                                       '--setting-sources', '', '--tools', ','.join(CLAUDE_TOOLS),
                                       '--allowedTools', ','.join(CLAUDE_TOOLS), '--permission-prompts', 'none',
                                       '--max-turns', str(MAX_TURNS), prompt]
                        command = [strace, '-f', '-e', 'trace=connect', '-o', str(directory/'connect.log')] + command
                        row['command'] = command
                        try:
                            with (directory/'stdout.log').open('w') as stdout, (directory/'stderr.log').open('w') as stderr:
                                row['exitCode'] = run_child(command, workspace, env, stdout, stderr)
                            if harness == 'LocalLM':
                                result = json.loads((directory/'result.json').read_text())
                                emitted = []
                                answer = '\n'.join(m['content'] for m in result['messages'] if m['role']=='assistant')
                                row.update(error=result['error'], turnEnd=result['turnEnd'],
                                           allowedTools=result['allowedTools'])
                                turn_end = result['turnEnd'] or {}
                                row['completed'] = turn_end.get('status') == 'complete' and turn_end.get('partial') is False
                            else:
                                emitted, result = read_claude_run(directory/'stdout.log')
                                answer = result.get('result', '')
                                row.update(error=result.get('is_error'), stopReason=result.get('stop_reason'),
                                           completed=result.get('subtype') == 'success' and not result.get('is_error'),
                                           allowedTools=CLAUDE_TOOLS, resultType=result.get('subtype'))
                            row['answer'] = answer
                            row.update(grade(answer, expected))
                            row.update(tool_operations(harness, result, emitted))
                        except subprocess.TimeoutExpired:
                            row.update(error=f'Run exceeded {TIMEOUT_SECONDS} seconds', timedOut=True)
                            if harness == 'Claude Code':
                                try:
                                    emitted, result = read_claude_run(directory/'stdout.log', timed_out=True)
                                    row.update(tool_operations(harness, result, emitted))
                                except (OSError, ValueError, TypeError, KeyError) as error:
                                    row['toolTraceError'] = str(error)
                        except (OSError, ValueError, TypeError, KeyError, StopIteration, subprocess.SubprocessError) as error:
                            row['error'] = str(error)
                        row['elapsedMs'] = (time.monotonic()-started)*1000
                        with condition:
                            drained = condition.wait_for(lambda: pending == 0, timeout=190)
                            if not drained:
                                raise TimeoutError('The owned model did not drain before the next run')
                            active = None
                            own = [record for record in records if record['run']==name and record.get('forwarded') and record['generation']]
                            own_errors = [record['error'] for record in records
                                          if record['run']==name and not record.get('forwarded') and 'error' in record]
                            (output/'requests.json').write_text(json.dumps(records, indent=2))
                        row['modelRequests'] = len(own)
                        row['tokenCountRequests'] = sum(1 for record in records if record['run'] == name
                                                        and record.get('forwarded') and not record['generation'])
                        row['setupViolations'] = own_errors
                        row['usageCoverage'] = {}
                        for key in ['inputTokens', 'cachedTokens', 'uncachedInputTokens', 'outputTokens', 'modelIssuedToolCalls']:
                            values = [record.get('usage', {}).get(key) for record in own]
                            row['usageCoverage'][key] = sum(value is not None for value in values)
                            row[key] = sum(values) if values and all(value is not None for value in values) else None
                        row['firstInputTokens'] = own[0].get('usage', {}).get('inputTokens') if own else None
                        row['firstCachedTokens'] = own[0].get('usage', {}).get('cachedTokens') if own else None
                        row['coldCacheVerified'] = row['firstCachedTokens'] == 0 if temperature == 'cold' else None
                        row['advertisedTools'] = sorted({
                            (tool.get('function') or tool)['name'] for record in own
                            for tool in record['request'].get('tools', [])
                        })
                        row['cacheHitRate'] = row['cachedTokens']/row['inputTokens'] if row['inputTokens'] and row['cachedTokens'] is not None else None
                        row['filesUnchanged'] = initial == snapshot(workspace)
                        if (directory/'connect.log').exists():
                            connections = (directory/'connect.log').read_text()
                            row['connectedIPv4'] = sorted(set(re.findall(r'inet_addr\("([^"]+)"\)', connections)))
                            row['connectedIPv6'] = sorted(set(re.findall(r'inet_pton\(AF_INET6, "([^"]+)"', connections)))
                        row['success'] = (row.get('exitCode') == 0 and not row.get('error') and row.get('completed', False)
                                          and row['formatCorrect'] and row['factsCorrect'] and row['filesUnchanged']
                                          and not row.get('permissionDenials') and not own_errors
                                          and row.get('successfulToolOperations', 0) > 0)
                        results.append(row)
                        (output/'report.json').write_text(json.dumps(dict(schemaVersion='locallm.harness-comparison/2',
                            model=MODEL, repetitions=args.repetitions, maxTurns=MAX_TURNS, maxOutputTokens=MAX_OUTPUT,
                            timeoutSeconds=TIMEOUT_SECONDS, graderCasesPassed=grader_cases,
                            bufferingProxy=True, results=results), indent=2)+'\n')
                        print(name+': '+('PASS' if row['success'] else 'FAIL')+', requests='+str(row['modelRequests']), flush=True)
    finally:
        active = None
        proxy.shutdown()
        proxy.server_close()
        fixtures.cleanup()


if __name__ == '__main__':
    main()
