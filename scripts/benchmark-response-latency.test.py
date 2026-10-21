"""Protocol, correctness and truthful-aggregation regressions for the diagnostic."""
import importlib.util
import io
import json
import os
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('response_latency', Path(__file__).with_name('benchmark-response-latency.py'))
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


def response(events, done=True):
    wire = ''.join('data: ' + json.dumps(event) + '\n\n' for event in events)
    if done:
        wire += 'data: [DONE]\n\n'
    return io.BytesIO(wire.encode())


def delta(content='', **extra):
    return {'choices': [{'delta': {'content': content, **extra}}]}


STOP = {'choices': [{'delta': {}, 'finish_reason': 'stop'}]}
OPTIONS = SimpleNamespace(max_tokens=64, request_timeout=3)


class DiagnosticTests(unittest.TestCase):
    def test_correctness_checks_reject_incomplete_and_wrong_answers(self):
        self.assertTrue(benchmark.check_answer('simple', '4'))
        self.assertFalse(benchmark.check_answer('simple', 'Let me think: 4 or 5'))
        self.assertTrue(benchmark.check_answer('research', 'Alpha is 6 ms faster [A][B].'))
        self.assertFalse(benchmark.check_answer('research', 'Beta is 6 ms faster [A][B].'))
        self.assertFalse(benchmark.check_answer('research', 'Alpha is 6 ms faster.'))
        self.assertTrue(benchmark.check_answer('coding', '```python\ndef add(a, b):\n    return a + b\n```'))
        for code in ('def add(a,b): return a-b', 'import os\ndef add(a,b): return a+b',
                     '@decorator\ndef add(a,b): return a+b', 'def add(a=execute(), b=0): return a+b',
                     'def add(a: execute(), b): return a+b', 'def add(a,b):\n execute()\n return a+b'):
            self.assertFalse(benchmark.check_answer('coding', code), code)

    def test_reasoning_tags_do_not_leak_at_any_chunk_boundary(self):
        source = '<think>private reasoning</think>4'
        for split in range(len(source) + 1):
            parser = benchmark.ThoughtFilter()
            chunks = [parser.feed(source[:split]), parser.feed(source[split:]), parser.feed('', final=True)]
            self.assertEqual(''.join(item[0] for item in chunks), '4', split)
            self.assertEqual(''.join(item[1] for item in chunks), 'private reasoning', split)
        parser = benchmark.ThoughtFilter()
        self.assertEqual(parser.feed('less < than'), ('less < than', ''))

    def test_fragmented_native_tool_call_is_executed_only_as_fixture(self):
        payloads = []
        def request(_path, payload, _timeout):
            payloads.append(payload)
            if len(payloads) == 1:
                return response([delta(tool_calls=[{'index': 0, 'id': 'call_test', 'function': {'name': 'read_', 'arguments': '{"id":'}}]),
                                 delta(tool_calls=[{'index': 0, 'function': {'name': 'fixture', 'arguments': '"simple"}'}}]),
                                 {'choices': [{'delta': {}, 'finish_reason': 'tool_calls'}]}])
            return response([delta('4'), STOP])
        result = benchmark.run_case(request, 'simple', 'auto', True, 'warm', 1, {}, OPTIONS)
        self.assertTrue(result['correct'])
        self.assertEqual(len(result['rounds']), 2)
        self.assertEqual(payloads[1]['messages'][-1], {'role': 'tool', 'tool_call_id': 'call_test', 'content': '2 + 2 = 4.'})
        self.assertEqual(payloads[0]['tools'], payloads[1]['tools'])
        self.assertTrue(payloads[1]['cache_prompt'])

    def test_arbitrary_tool_is_never_executed(self):
        def request(*_args):
            return response([delta(tool_calls=[{'index': 0, 'id': 'bad', 'function': {'name': 'execute_command', 'arguments': '{"command":"echo unsafe"}'}}]),
                             {'choices': [{'delta': {}, 'finish_reason': 'tool_calls'}]}])
        result = benchmark.run_case(request, 'simple', 'quick', True, 'cold', 1, {}, OPTIONS)
        self.assertFalse(result['complete'])
        self.assertFalse(result['correct'])
        self.assertIn('declared synthetic fixture', result['error'])

    def test_missing_done_and_reasoning_only_limit_fail(self):
        result = benchmark.run_case(lambda *_: response([delta('4'), STOP], done=False), 'simple', 'auto', False, 'cold', 1, {}, OPTIONS)
        self.assertFalse(result['complete'])
        self.assertIn('completion marker', result['error'])
        result = benchmark.run_case(lambda *_: response([delta(reasoning_content='Need to think'),
            {'choices': [{'delta': {}, 'finish_reason': 'length'}]}]), 'simple', 'auto', False, 'cold', 1, {}, OPTIONS)
        self.assertFalse(result['correct'])
        self.assertIsNone(result['firstAnswerMs'])
        self.assertIsNotNone(result['firstTokenMs'])
        self.assertIn('budget exhausted', result['error'])

    def test_first_answer_excludes_fragmented_reasoning(self):
        result = benchmark.run_case(lambda *_: response([delta('<thi'), delta('nk>hidden</th'), delta('ink>4'), STOP]),
                                    'simple', 'quick', False, 'cold', 1, {}, OPTIONS)
        self.assertTrue(result['correct'])
        self.assertEqual(result['answer'], '4')
        self.assertEqual(result['reasoning'], 'hidden')
        self.assertLessEqual(result['firstTokenMs'], result['firstAnswerMs'])

    def test_summary_excludes_priming_and_failed_latencies_but_counts_failures(self):
        common = {'task': 'simple', 'mode': 'quick', 'tools': False, 'cache': 'warm'}
        records = [{**common, 'complete': True, 'correct': True, 'firstAnswerMs': 10, 'totalMs': 20},
                   {**common, 'complete': True, 'correct': False, 'firstAnswerMs': 30, 'totalMs': 60},
                   {**common, 'complete': False, 'correct': False, 'firstAnswerMs': 1000, 'totalMs': 2000},
                   {**common, 'cache': 'prime', 'complete': True, 'correct': True, 'firstAnswerMs': 9999, 'totalMs': 9999}]
        summary = benchmark.summarize(records)[0]
        self.assertEqual(summary['samples'], 3)
        self.assertEqual(summary['completed'], 2)
        self.assertAlmostEqual(summary['correctnessRate'], 1/3)
        self.assertEqual(summary['firstAnswerMs'], {'n': 2, 'median': 20, 'p95': 30})
        self.assertEqual(summary['firstTokenMs'], {'n': 0, 'median': None, 'p95': None})

    def test_sse_multiline_and_comments(self):
        wire = io.BytesIO(b':ping\n\ndata: {"ok":\ndata: true}\n\nevent: done\ndata: [DONE]\n\n')
        values = list(benchmark.sse_values(wire))
        self.assertEqual(json.loads(values[0]), {'ok': True})
        self.assertEqual(values[1], '[DONE]')

    @unittest.skipUnless(os.name == 'posix', 'POSIX runtime process fixture')
    def test_runtime_startup_failure_persists_failure_and_cleanup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / 'runtime'
            runtime.write_text('#!/bin/sh\nif [ "$1" = "--version" ]; then echo fixture; exit 0; fi\nexit 7\n')
            runtime.chmod(0o700)
            model = root / 'fixture.gguf'
            model.write_bytes(b'fixture')
            with patch('sys.stdout', io.StringIO()):
                code = benchmark.main(['--runtime', str(runtime), '--model', str(model), '--output', str(root/'result')])
            result = json.loads((root/'result'/'measurements.json').read_text())
            self.assertEqual(code, 1)
            self.assertFalse(result['success'])
            self.assertTrue(result['processStopped'])
            self.assertIn('exited 7', result['error'])

    @unittest.skipUnless(os.name == 'posix', 'POSIX runtime process fixture')
    def test_interruption_stops_owned_process_and_preserves_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / 'runtime'
            runtime.write_text('#!/bin/sh\nif [ "$1" = "--version" ]; then echo fixture; exit 0; fi\nexec sleep 30\n')
            runtime.chmod(0o700)
            model = root / 'fixture.gguf'
            model.write_bytes(b'fixture')
            with patch('urllib.request.OpenerDirector.open', side_effect=InterruptedError('test interruption')), patch('sys.stdout', io.StringIO()):
                code = benchmark.main(['--runtime', str(runtime), '--model', str(model), '--output', str(root/'result')])
            result = json.loads((root/'result'/'measurements.json').read_text())
            self.assertEqual(code, 1)
            self.assertFalse(result['success'])
            self.assertTrue(result['processStopped'])
            self.assertEqual(result['error'], 'test interruption')


if __name__ == '__main__':
    unittest.main()
