#!/usr/bin/env python3
"""Bounded local-runtime response diagnostic; never executes generated code.

Compare synthetic simple, coding and grounded-research fixtures with tool
schemas on/off and requested cold/warm prompt caches. This measures HTTP client
observations, not application UI paint, live web quality, or a causal speedup.
"""
import argparse
import ast
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import secrets
import signal
import socket
import statistics
import subprocess
import sys
import time
import urllib.error
import urllib.request

FIXTURES = {
    'simple': {
        'prompt': 'What is 2 + 2? Reply with the number only.',
        'reference': '2 + 2 = 4.',
    },
    'coding': {
        'prompt': 'Write only a Python function add(a, b) that returns a + b. No explanation.',
        'reference': 'def add(a, b):\n    return a + b\n',
    },
    'research': {
        'prompt': 'Use only these synthetic test excerpts: [A] Alpha latency is 12 ms. '
                  '[B] Beta latency is 18 ms. Which is faster and by how much? '
                  'Reply exactly: Alpha is 6 ms faster [A][B].',
        'reference': '[A] Alpha latency is 12 ms. [B] Beta latency is 18 ms.',
    },
}


def check_answer(task, answer):
    """Deliberately narrow fixture checks, not an open-ended quality grader."""
    value = answer.strip()
    if task == 'simple':
        return value == '4'
    if task == 'research':
        return bool(re.fullmatch(r'Alpha is 6 ms faster\s*\[A\]\s*\[B\][.!]?', value, re.I))
    value = re.sub(r'^```(?:python)?\s*\n?', '', value)
    value = re.sub(r'\n?```$', '', value)
    try:
        tree = ast.parse(value)
    except (SyntaxError, ValueError, RecursionError):
        return False
    if len(tree.body) != 1 or not isinstance(tree.body[0], ast.FunctionDef):
        return False
    function = tree.body[0]
    body = function.body
    if body and isinstance(body[0], ast.Expr) and isinstance(body[0].value, ast.Constant) and isinstance(body[0].value.value, str):
        body = body[1:]
    args = function.args
    if function.name != 'add' or function.decorator_list or args.defaults or args.kw_defaults or args.vararg or args.kwarg or args.kwonlyargs:
        return False
    if [arg.arg for arg in args.posonlyargs + args.args] != ['a', 'b'] or len(body) != 1 or not isinstance(body[0], ast.Return):
        return False
    if function.returns or getattr(function, 'type_params', []) or any(arg.annotation for arg in args.posonlyargs + args.args):
        return False
    expression = body[0].value
    return (isinstance(expression, ast.BinOp) and isinstance(expression.op, ast.Add)
            and isinstance(expression.left, ast.Name) and expression.left.id == 'a'
            and isinstance(expression.right, ast.Name) and expression.right.id == 'b')


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)] if ordered else None


def summarize(records):
    groups = {}
    for record in records:
        if record['cache'] == 'prime':
            continue
        key = (record['task'], record['mode'], record['tools'], record['cache'])
        groups.setdefault(key, []).append(record)
    result = []
    for (task, mode, tools, cache), rows in groups.items():
        # Failed/incomplete generations count in correctness, never in successful latency.
        completed = [row for row in rows if row.get('complete')]
        item = {'task': task, 'mode': mode, 'tools': tools, 'cache': cache,
                'samples': len(rows), 'completed': len(completed),
                'correct': sum(bool(row.get('correct')) for row in rows),
                'correctnessRate': sum(bool(row.get('correct')) for row in rows) / len(rows)}
        for field in ('firstTokenMs', 'firstAnswerMs', 'totalMs'):
            values = [row[field] for row in completed if isinstance(row.get(field), (int, float)) and math.isfinite(row[field]) and row[field] >= 0]
            item[field] = {'n': len(values), 'median': statistics.median(values) if values else None,
                           'p95': percentile(values, .95)}
        item['note'] = 'Small descriptive sample; no statistical or causal speedup claim.'
        result.append(item)
    return result


def sse_values(response):
    data = []
    for raw in response:
        line = raw.decode('utf-8').rstrip('\r\n')
        if not line:
            if data:
                yield '\n'.join(data)
                data = []
        elif line.startswith('data:'):
            data.append(line[5:].lstrip(' '))
    if data:
        yield '\n'.join(data)


class ThoughtFilter:
    """Separate <think> markers, including markers split across SSE events."""
    def __init__(self):
        self.pending = ''
        self.inside = False

    def feed(self, text, final=False):
        self.pending += text
        answer, thought = '', ''
        while self.pending:
            marker = '</think>' if self.inside else '<think>'
            index = self.pending.find(marker)
            if index >= 0:
                segment, self.pending = self.pending[:index], self.pending[index + len(marker):]
                if self.inside: thought += segment
                else: answer += segment
                self.inside = not self.inside
                continue
            keep = 0 if final else next((n for n in range(min(len(marker)-1, len(self.pending)), 0, -1) if self.pending.endswith(marker[:n])), 0)
            segment = self.pending[:-keep] if keep else self.pending
            self.pending = self.pending[-keep:] if keep else ''
            if self.inside: thought += segment
            else: answer += segment
            break
        return answer, thought


def tool_schema(task):
    return {'type': 'function', 'function': {'name': 'read_fixture',
        'description': 'Read the synthetic reference for this diagnostic only. No files, network or code execution.',
        'parameters': {'type': 'object', 'properties': {'id': {'type': 'string', 'enum': [task]}},
                       'required': ['id'], 'additionalProperties': False}}}


def stop_runtime(process):
    if process is None or process.poll() is not None:
        return
    if os.name == 'nt':
        # Only the numeric PID created by this runner is targeted.
        subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'],
                       capture_output=True, timeout=10, check=False)
    else:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        if os.name == 'nt':
            process.kill()
        else:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        process.wait(timeout=5)


def run_case(request, task, mode, tools, cache, sample, control, options):
    started = time.monotonic()
    record = {'task': task, 'mode': mode, 'tools': tools, 'cache': cache, 'sample': sample,
              'requestedControl': control, 'firstTokenMs': None, 'firstAnswerMs': None,
              'answer': '', 'reasoning': '', 'rounds': [], 'complete': False, 'correct': False}
    messages = [{'role': 'system', 'content': 'Answer accurately and concisely. Follow the requested output format.'},
                {'role': 'user', 'content': FIXTURES[task]['prompt']}]
    try:
        for round_index in range(3):
            payload = {'messages': messages, 'temperature': 0, 'seed': 42, 'max_tokens': options.max_tokens,
                       'stream': True, 'stream_options': {'include_usage': True}, 'timings': True,
                       'cache_prompt': cache != 'cold', 'id_slot': 0, **control}
            if tools:
                payload['tools'] = [tool_schema(task)]
                payload['tool_choice'] = 'none' if round_index == 2 else 'auto'
            turn = {'content': '', 'reasoning': '', 'toolCalls': {}, 'done': False}
            thought_filter = ThoughtFilter()
            with request('/v1/chat/completions', payload, options.request_timeout) as response:
                for data in sse_values(response):
                    if time.monotonic() - started > options.request_timeout * 3:
                        raise TimeoutError('Case exceeded total deadline')
                    if data == '[DONE]':
                        turn['done'] = True
                        break
                    value = json.loads(data)
                    if value.get('error'): raise RuntimeError(str(value['error']))
                    choice = (value.get('choices') or [{}])[0]
                    delta = choice.get('delta', {})
                    raw_content = delta.get('content') or ''
                    content, thought = thought_filter.feed(raw_content)
                    thought += delta.get('reasoning_content') or delta.get('reasoning') or ''
                    elapsed = (time.monotonic() - started) * 1000
                    if record['firstTokenMs'] is None and (raw_content or thought or delta.get('tool_calls')): record['firstTokenMs'] = elapsed
                    if record['firstAnswerMs'] is None and content.strip(): record['firstAnswerMs'] = elapsed
                    turn['content'] += content
                    turn['reasoning'] += thought
                    for call in delta.get('tool_calls') or []:
                        index = call.get('index', 0)
                        if not isinstance(index, int) or index < 0 or index > 15: raise ValueError('Invalid tool-call index')
                        partial = turn['toolCalls'].setdefault(index, {'id': '', 'type': 'function', 'function': {'name': '', 'arguments': ''}})
                        if call.get('id'): partial['id'] = call['id']
                        for field in ('name', 'arguments'):
                            partial['function'][field] += call.get('function', {}).get(field) or ''
                    if choice.get('finish_reason'): turn['finishReason'] = choice['finish_reason']
                    if value.get('usage'): turn['usage'] = value['usage']
                    if value.get('timings'): turn['timings'] = value['timings']
            content, thought = thought_filter.feed('', final=True)
            turn['content'] += content
            turn['reasoning'] += thought
            if content.strip() and record['firstAnswerMs'] is None: record['firstAnswerMs'] = (time.monotonic()-started)*1000
            record['reasoning'] += turn['reasoning']
            calls = list(turn['toolCalls'].values())
            turn['toolCalls'] = calls
            record['rounds'].append(turn)
            if not turn['done']: raise RuntimeError('Missing SSE completion marker')
            if turn.get('finishReason') == 'length': raise RuntimeError('Response token budget exhausted')
            if not calls:
                record['answer'] = turn['content']
                record['complete'] = turn.get('finishReason') == 'stop' and bool(turn['content'].strip())
                record['correct'] = record['complete'] and check_answer(task, turn['content'])
                break
            if not tools or round_index == 2: raise RuntimeError('Unexpected or excessive tool call')
            messages.append({'role': 'assistant', 'content': turn['content'], 'tool_calls': calls})
            for call in calls:
                arguments = json.loads(call['function']['arguments'])
                if not call['id'] or call['function']['name'] != 'read_fixture' or arguments != {'id': task}:
                    raise ValueError('Diagnostic accepts only the declared synthetic fixture tool')
                messages.append({'role': 'tool', 'tool_call_id': call['id'], 'content': FIXTURES[task]['reference']})
    except Exception as error:
        record['error'] = str(error)
    record['totalMs'] = (time.monotonic() - started) * 1000
    return record


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', type=Path, required=True)
    parser.add_argument('--model', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--samples', type=int, default=3, choices=range(1, 21))
    parser.add_argument('--modes', nargs='+', choices=['auto', 'quick', 'think'], default=['auto', 'quick'])
    parser.add_argument('--tasks', nargs='+', choices=list(FIXTURES), default=list(FIXTURES))
    parser.add_argument('--max-tokens', type=int, default=160)
    parser.add_argument('--context', type=int, default=4096)
    parser.add_argument('--threads', type=int, default=2)
    parser.add_argument('--gpu-layers', type=int, default=0)
    parser.add_argument('--request-timeout', type=int, default=90)
    options = parser.parse_args(argv)
    if not options.runtime.is_file() or not options.model.is_file(): parser.error('Runtime and model must be existing local files')
    if not 1 <= options.max_tokens <= 4096 or not 2048 <= options.context <= 32768 or not 1 <= options.threads <= 64 or not 1 <= options.request_timeout <= 300:
        parser.error('Use tokens 1..4096, context 2048..32768, threads 1..64 and timeout 1..300 seconds')
    options.output.mkdir(parents=True, exist_ok=True)
    if (options.output / 'measurements.json').exists(): parser.error('Output already contains a report; choose a new directory')
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    key = secrets.token_urlsafe(32)
    env = {name: value for name, value in os.environ.items() if not name.startswith('LLAMA_') and name != 'MTMD_BACKEND_DEVICE'}
    env['LLAMA_API_KEY'] = key
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def request(path, payload=None, timeout=5):
        req = urllib.request.Request(f'http://127.0.0.1:{port}{path}', data=None if payload is None else json.dumps(payload).encode(),
                                     headers={'Authorization': f'Bearer {key}', 'Content-Type': 'application/json'})
        return opener.open(req, timeout=timeout)
    args = [str(options.runtime.resolve()), '--model', str(options.model.resolve()), '--host', '127.0.0.1', '--port', str(port),
            '--ctx-size', str(options.context), '--threads', str(options.threads), '--threads-batch', str(options.threads),
            '--batch-size', '256', '--ubatch-size', '64', '--parallel', '1', '--n-gpu-layers', str(options.gpu_layers),
            '--fit', 'off', '--jinja', '--no-webui', '--no-agent', '--no-mmproj', '--cache-type-k', 'q8_0', '--cache-type-v', 'q8_0', '--flash-attn', 'on']
    if options.gpu_layers == 0: args.append('--no-kv-offload')
    report = {'schemaVersion': 1, 'date': time.strftime('%Y-%m-%dT%H:%M:%S%z'), 'runtime': str(options.runtime.resolve()),
              'model': str(options.model.resolve()), 'config': {name: getattr(options, name) for name in ('samples', 'modes', 'tasks', 'max_tokens', 'context', 'threads', 'gpu_layers')},
              'runnerSha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'hardware': {'system': platform.system(), 'release': platform.release(), 'machine': platform.machine(),
                           'processor': platform.processor(), 'logicalCpus': os.cpu_count()},
              'modelFile': {'bytes': options.model.stat().st_size, 'modifiedNs': options.model.stat().st_mtime_ns},
              'boundaries': ['Synthetic fixture correctness only; research is not live web retrieval.',
                             'First answer is observed by the HTTP client, before UI rendering.',
                             'Cold means requested cache_prompt=false, not cold disk or fresh process.',
                             'Warm is primed with an identical request; inspect reported cache counters.',
                             'p95 is nearest-rank; small samples do not establish a causal speedup.'],
              'records': [], 'processStopped': False, 'success': False}
    process = None
    def interrupted(_signum, _frame): raise InterruptedError('Diagnostic interrupted')
    previous = signal.signal(signal.SIGTERM, interrupted)
    try:
        with (options.output / 'runtime.log').open('x') as log:
            version = subprocess.run([str(options.runtime.resolve()), '--version'], env=env, capture_output=True, text=True, timeout=15)
            report['runtimeVersion'] = (version.stdout + version.stderr).strip()[-1024:]
            started = time.monotonic()
            group = {'creationflags': subprocess.CREATE_NEW_PROCESS_GROUP} if os.name == 'nt' else {'start_new_session': True}
            process = subprocess.Popen(args, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log, **group)
            while time.monotonic() - started < 120:
                if process.poll() is not None: raise RuntimeError(f'Runtime exited {process.returncode}; inspect runtime.log')
                try:
                    with request('/health', timeout=1) as response:
                        if json.load(response).get('status') == 'ok': break
                except (urllib.error.URLError, TimeoutError, ConnectionError): pass
                time.sleep(.25)
            else: raise TimeoutError('Runtime not ready after 120 seconds')
            report['loadSeconds'] = time.monotonic() - started
            with request('/props') as response: props = json.load(response)
            report['templateCaps'] = props.get('chat_template_caps')
            template = props.get('chat_template') or ''
            # Probe effective built-in templates too; absence of GGUF metadata is not lack of support.
            probe_messages = [{'role': 'user', 'content': 'Diagnostic.'}]
            prompts = {}
            for enabled in (False, True):
                with request('/apply-template', {'messages': probe_messages, 'chat_template_kwargs': {'enable_thinking': enabled}}) as response:
                    prompts[str(enabled)] = json.load(response).get('prompt')
            report['thinkingTemplateChanges'] = bool(prompts['False'] and prompts['True'] and prompts['False'] != prompts['True'])
            controls = {'auto': {}}
            if report['thinkingTemplateChanges'] or 'enable_thinking' in template:
                controls.update({'quick': {'chat_template_kwargs': {'enable_thinking': False}}, 'think': {'chat_template_kwargs': {'enable_thinking': True}}})
            elif (props.get('chat_template_caps') or {}).get('supports_reasoning_effort'):
                controls.update({'quick': {'chat_template_kwargs': {'reasoning_effort': 'low'}}, 'think': {'chat_template_kwargs': {'reasoning_effort': 'high'}}})
            report['unsupportedModes'] = [mode for mode in options.modes if mode not in controls]
            for mode in dict.fromkeys(options.modes):
                if mode not in controls: continue
                for task in dict.fromkeys(options.tasks):
                    for tools in (False, True):
                        for sample in range(1, options.samples + 1):
                            for cache in ('cold', 'prime', 'warm'):
                                record = run_case(request, task, mode, tools, cache, sample, controls[mode], options)
                                report['records'].append(record)
                                print(json.dumps({key: record.get(key) for key in ('task', 'mode', 'tools', 'cache', 'sample', 'complete', 'correct', 'firstAnswerMs', 'totalMs', 'error')}), flush=True)
            report['success'] = not report['unsupportedModes'] and all(row['complete'] and row['correct'] for row in report['records'])
    except (Exception, KeyboardInterrupt) as error:
        report['error'] = str(error) or 'Interrupted'
    finally:
        stop_runtime(process)
        report['processStopped'] = process is None or process.poll() is not None
        report['summary'] = summarize(report['records'])
        path = options.output / 'measurements.json'
        temporary = path.with_suffix('.json.tmp')
        temporary.write_text(json.dumps(report, indent=2) + '\n')
        temporary.replace(path)
        signal.signal(signal.SIGTERM, previous)
    print(json.dumps({'report': str(path.resolve()), 'success': report['success'], 'processStopped': report['processStopped']}), flush=True)
    return 0 if report['success'] else 1


if __name__ == '__main__':
    sys.exit(main())
