import { expect, it } from 'vitest';
import { EvidenceExtractor } from './extractor';
import type { EvidenceChunk } from '../types';

it('does not select a fact just because it shares question stop words', () => {
  const chunk = { id: 'c1', text: 'When the ES module is loaded, the returned object is what gets exported as a namespace object.', url: 'https://nodejs.org/' } as EvidenceChunk;
  expect(new EvidenceExtractor().deterministicExtract('What is the newest stable release of React?', chunk, 'S1')).toEqual([]);
});

it('requires the subject when generic release language also matches', () => {
  const chunk = { id: 'c1', title: 'Python release', text: 'Python 3.13.0 is the newest stable release of the Python programming language.', url: 'https://python.org/' } as EvidenceChunk;
  expect(new EvidenceExtractor().deterministicExtract('What is the newest stable release of React?', chunk, 'S1')).toEqual([]);
});
