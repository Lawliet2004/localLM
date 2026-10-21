#!/usr/bin/env python3
"""Measure an installed llama.cpp runtime without changing application settings.

Starts an isolated loopback server, replays an identical streamed request twice,
records backend timings, monotonic first-token latency, RSS and GPU memory, then
terminates its process group. Process cold start does not imply cold disk cache.
"""
import argparse
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import sqlite3
import subprocess
import time
import urllib.error
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--preferences-db', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cpu', action='store_true', help='Measure CPU fallback with a 4096-token context and no projector.')
    parser.add_argument('--tool-probe', action='store_true', help='Also request a structured diagnostic call; no tool is executed.')
    options = parser.parse_args()
    options.output.mkdir(parents=True, exist_ok=True)
    with sqlite3.connect(options.preferences_db.resolve().as_uri() + '?mode=ro', uri=True) as db:
        settings = {key: json.loads(value) for key, value in db.execute("SELECT key,value FROM settings WHERE key IN ('preferences','runtime')")}
    prefs, config = settings['preferences'], settings['runtime']
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    key = secrets.token_urlsafe(32)
    env = {name: value for name, value in os.environ.items() if not name.startswith('LLAMA_') and name != 'MTMD_BACKEND_DEVICE'}
    env['LLAMA_API_KEY'] = key
    args = [prefs['runtimePath'], '--model', prefs['modelPath'], '--ctx-size', str(4096 if options.cpu else config['contextLength']), '--threads', str(config['cpuThreads']), '--batch-size', str(config['batchSize']), '--ubatch-size', str(config['microBatchSize']), '--flash-attn', 'on' if config['flashAttention'] else 'off', '--cache-type-k', config['cacheTypeK'], '--cache-type-v', config['cacheTypeV'], '--parallel', '1', '--host', '127.0.0.1', '--port', str(port), '--jinja', '--no-webui']
    help_text = subprocess.run([prefs['runtimePath'], '--help'], env=env, capture_output=True, text=True, timeout=20).stdout
    if '--no-agent' in help_text:
        args += ['--no-agent']
    layers = 0 if options.cpu else config['gpuLayers']
    if '--fit' in help_text:
        args += ['--fit', 'on' if layers < 0 else 'off']
    if layers >= 0:
        args += ['--n-gpu-layers', str(layers)]
    if options.cpu or not config['offloadKvCache']:
        args += ['--no-kv-offload']
    if not config['mmap']:
        args += ['--no-mmap']
    if not options.cpu and config.get('visionEnabled', True) and prefs.get('projectorPath'):
        args += ['--mmproj', prefs['projectorPath'], '--mmproj-offload' if config.get('offloadProjector', True) else '--no-mmproj-offload']
    else:
        args += ['--no-mmproj']
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(path, payload=None, timeout=5):
        req = urllib.request.Request(f'http://127.0.0.1:{port}{path}', data=None if payload is None else json.dumps(payload).encode(), headers={'Authorization': f'Bearer {key}', 'Content-Type': 'application/json'})
        return opener.open(req, timeout=timeout)

    def memory(pid):
        result = {}
        status = Path(f'/proc/{pid}/status')
        if status.exists():
            for line in status.read_text().splitlines():
                if line.startswith(('VmRSS:', 'VmHWM:')):
                    result[line.split(':')[0] + 'KiB'] = int(line.split()[1])
        try:
            gpu = subprocess.run(['nvidia-smi', '--query-gpu=name,memory.used,memory.total', '--format=csv,noheader,nounits'], capture_output=True, text=True, timeout=5)
            result['gpuMemorySnapshot'] = gpu.stdout.strip()
        except (OSError, subprocess.TimeoutExpired):
            pass
        return result

    report = {'testedAt': time.strftime('%Y-%m-%dT%H:%M:%S%z'), 'runtime': prefs['runtimePath'], 'model': prefs['modelPath'], 'cpuFallback': options.cpu, 'savedConfig': config, 'rounds': [], 'beforeLoad': memory(os.getpid())}
    process = None
    started = time.monotonic()
    try:
        with (options.output / 'runtime.log').open('w') as log:
            process = subprocess.Popen(args, env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
            while time.monotonic() - started < 240:
                if process.poll() is not None:
                    raise RuntimeError(f'Runtime exited {process.returncode}; inspect runtime.log')
                try:
                    with request('/health') as response:
                        if json.load(response).get('status') == 'ok':
                            break
                except (urllib.error.URLError, TimeoutError, ConnectionError):
                    pass
                time.sleep(0.25)
            else:
                raise TimeoutError('Runtime was not healthy within 240 seconds')
            report['loadSeconds'] = time.monotonic() - started
            report['afterLoad'] = memory(process.pid)
            payload = {'messages': [{'role': 'system', 'content': 'Answer concisely. ' + 'This is a repeatable cache and timing fixture. ' * 70}, {'role': 'user', 'content': 'List the integers from 1 to 15, separated by spaces.'}], 'temperature': 0, 'max_tokens': 64, 'stream': True, 'cache_prompt': True, 'id_slot': 0, 'timings': True, 'stream_options': {'include_usage': True}, 'chat_template_kwargs': {'enable_thinking': False}}
            for label in ('cold_prompt', 'warm_identical_prompt'):
                round_start = time.monotonic()
                measured = {'label': label, 'firstTokenMs': None, 'response': '', 'reasoning': '', 'usage': None, 'timings': None, 'complete': False}
                with request('/v1/chat/completions', payload, timeout=180) as response:
                    for line in response:
                        if not line.startswith(b'data:'):
                            continue
                        value = line[5:].strip()
                        if value == b'[DONE]':
                            measured['complete'] = True
                            break
                        event = json.loads(value)
                        if event.get('error'):
                            raise RuntimeError(str(event['error']))
                        delta = (event.get('choices') or [{}])[0].get('delta', {})
                        text, thought = delta.get('content') or '', delta.get('reasoning_content') or ''
                        if measured['firstTokenMs'] is None and (text or thought or delta.get('tool_calls')):
                            measured['firstTokenMs'] = (time.monotonic() - round_start) * 1000
                        measured['response'] += text
                        measured['reasoning'] += thought
                        if event.get('usage'):
                            measured['usage'] = event['usage']
                        if event.get('timings'):
                            measured['timings'] = event['timings']
                measured['durationMs'] = (time.monotonic() - round_start) * 1000
                report['rounds'].append(measured)
                if not measured['complete'] or not measured['response']:
                    raise RuntimeError('Stream did not complete with an answer')
                print(json.dumps(measured), flush=True)
            report['afterInference'] = memory(process.pid)
            if options.tool_probe:
                with request('/v1/chat/completions', {'messages': [{'role': 'user', 'content': 'Call locallm_probe with value 42. Do not answer in prose.'}], 'tools': [{'type': 'function', 'function': {'name': 'locallm_probe', 'description': 'Return the supplied test value. This diagnostic never executes a tool.', 'parameters': {'type': 'object', 'properties': {'value': {'type': 'integer', 'const': 42}}, 'required': ['value'], 'additionalProperties': False}}}], 'tool_choice': {'type': 'function', 'function': {'name': 'locallm_probe'}}, 'temperature': 0, 'max_tokens': 256, 'stream': False, 'cache_prompt': False, 'chat_template_kwargs': {'enable_thinking': False}}, timeout=90) as response:
                    probe = json.load(response)
                calls = probe.get('choices', [{}])[0].get('message', {}).get('tool_calls') or []
                if len(calls) != 1 or calls[0].get('type') != 'function' or not calls[0].get('id') or calls[0].get('function', {}).get('name') != 'locallm_probe' or json.loads(calls[0]['function']['arguments']) != {'value': 42}:
                    raise RuntimeError('The model did not produce the required native structured diagnostic call')
                report['toolProbe'] = probe
                print('Native structured tool call verified; no tool executed.', flush=True)
            report['success'] = True
    except Exception as error:
        report['success'], report['error'] = False, str(error)
        raise
    finally:
        if process is not None and process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=5)
        report['processStopped'] = process is None or process.poll() is not None
        (options.output / 'measurements.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
