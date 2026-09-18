import { expect, it } from 'vitest';
import { comparablePath, samePath } from './pathUtils';

it('treats Windows device and regular paths as the same file', () => {
  expect(samePath('\\\\?\\C:\\Models\\Mini.gguf', 'c:/models/mini.gguf')).toBe(true);
  expect(comparablePath('C:\\Models\\Mini.gguf\\')).toBe('c:/models/mini.gguf');
});

it('does not collapse different model files', () => {
  expect(samePath('C:\\Models\\Mini.gguf', 'C:\\Models\\Zaya.gguf')).toBe(false);
  expect(samePath('', 'C:\\Models\\Mini.gguf')).toBe(false);
});
