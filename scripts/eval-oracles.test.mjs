import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import test from 'node:test';
import { checkHohmannTransferTime, evaluateKnownAnswer, HOHMANN_TRANSFER_SECONDS } from './eval-oracles.mjs';

const complete = content => [{ role: 'assistant', status: 'complete', content }];

test('Hohmann oracle computes the half-orbit transfer time', () => {
  assert.ok(Math.abs(HOHMANN_TRANSFER_SECONDS - 18_990.211638) < 0.001);
  assert.equal(checkHohmannTransferTime(complete('Transfer time: 18,990 s (5.28 hours)')).verdict, 'pass');
  assert.equal(checkHohmannTransferTime(complete('Transfer time: 1.90×10⁴ s')).verdict, 'pass');
  assert.equal(checkHohmannTransferTime(complete('Transfer time: 1.90e4 s')).verdict, 'pass');
  assert.equal(checkHohmannTransferTime(complete('Transfer time: 37,980 s (10.55 hours)')).verdict, 'fail');
  assert.equal(checkHohmannTransferTime(complete('No transfer time computed.')).verdict, 'unparsed');
  assert.deepEqual(evaluateKnownAnswer(2, complete('anything')), {});
});

const tracePath = new URL('../eval-results/minicpm/04-hohmann-transfer.json', import.meta.url);
test('the saved MiniCPM Hohmann trace fails the numerical oracle', { skip: !existsSync(tracePath) }, () => {
  const trace = JSON.parse(readFileSync(tracePath, 'utf8'));
  const check = evaluateKnownAnswer(trace.testId, trace.messages).hohmannTransferTime;
  assert.equal(check.verdict, 'fail');
  assert.ok(check.observedSeconds.some(seconds => Math.abs(seconds - 37_980) < 100));
});
