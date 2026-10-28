#!/usr/bin/env python3
"""Exercise budget accounting only against isolated temporary ledgers."""
import concurrent.futures
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('evaluation', Path(__file__).with_name('harness-evaluation.py'))
evaluation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(evaluation)
# These test-local limits never touch the real evaluator process or ledger.
evaluation.REMOTE_LIMIT = 100
evaluation.TRIAL_LIMIT = 90


class Accounting(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.ledger = evaluation.Ledger(Path(self.directory.name) / 'tokens.sqlite')

    def charged(self):
        return self.ledger.report()[0]['chargedTokens']

    def test_missing_or_invalid_usage_keeps_the_reservation(self):
        for usage in [None, 'unavailable', {'total_tokens': -10}, {'total_tokens': True},
                      {'prompt_tokens': -5, 'completion_tokens': 10}, {'total_tokens': 1.5}]:
            with self.subTest(usage=usage):
                ledger = evaluation.Ledger(Path(self.directory.name) / f'case-{len(str(usage))}-{str(usage)}.sqlite')
                identity = ledger.reserve('remote', 'task', 80)
                ledger.settle(identity, usage, 200)
                self.assertEqual(ledger.report()[0]['chargedTokens'], 80)

    def test_valid_usage_releases_only_unused_reservation(self):
        identity = self.ledger.reserve('remote', 'first', 80)
        self.ledger.settle(identity, {'prompt_tokens': 10, 'completion_tokens': 5}, 200)
        self.assertEqual(self.charged(), 15)
        self.ledger.reserve('remote', 'second', 85)
        self.assertEqual(self.charged(), 100)
        with self.assertRaises(evaluation.BudgetDenied) as denied:
            self.ledger.reserve('remote', 'third', 1)
        self.assertEqual(denied.exception.scope, 'combined_remote')

    def test_concurrent_connections_cannot_admit_over_budget(self):
        def reserve(number):
            ledger = evaluation.Ledger(self.ledger.path)
            try:
                ledger.reserve('remote', str(number), 60)
                return True
            except evaluation.BudgetDenied:
                return False
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
            self.assertEqual(sum(executor.map(reserve, range(4))), 1)
        self.assertEqual(self.charged(), 60)

    def test_local_allowance_is_separate_and_denials_add_no_charge(self):
        self.ledger.reserve('remote', 'remote', 90)
        self.ledger.reserve('local', 'local', 90)
        with self.assertRaises(evaluation.BudgetDenied) as denied:
            self.ledger.reserve('local', 'local', 1)
        self.assertEqual(denied.exception.scope, 'trial')
        self.assertEqual(len(self.ledger.report()), 2)
        self.assertTrue(all(row['chargedTokens'] == 90 for row in self.ledger.report()))

    def test_usage_above_reservation_is_never_hidden(self):
        identity = self.ledger.reserve('remote', 'first', 80)
        self.ledger.settle(identity, {'total_tokens': 101}, 200)
        self.assertEqual(self.charged(), 101)
        with self.assertRaises(evaluation.BudgetDenied):
            self.ledger.reserve('remote', 'next', 1)


if __name__ == '__main__':
    unittest.main()
