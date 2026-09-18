import { expect, it } from 'vitest';
import { composeMessage, readAttachment } from './attachments';

it('includes the actual file contents as quoted reference data in the message', async () => {
  const file = new File(['name,value\nalpha,42'], 'results.csv', { type: 'text/csv' });
  const attachment = await readAttachment(file);
  const message = composeMessage('Explain this', [attachment]);
  expect(message).toContain('Explain this');
  expect(message).toContain('results.csv');
  expect(message).toContain('alpha,42');
  expect(message).toContain('reference data');
});

it('rejects unsupported, binary and oversized attachments with actionable errors', async () => {
  await expect(readAttachment(new File(['binary'], 'photo.png', { type: 'image/png' }))).rejects.toThrow(/not supported/);
  await expect(readAttachment(new File(['a\0b'], 'data.txt'))).rejects.toThrow(/UTF-8 text/);
  await expect(readAttachment(new File(['x'.repeat(256001)], 'large.txt'))).rejects.toThrow(/250 KB/);
});

it('bounds the combined request and leaves ordinary messages unchanged', () => {
  expect(composeMessage('hello', [])).toBe('hello');
  expect(() => composeMessage('x'.repeat(100000), [{ id: '1', name: 'large.txt', size: 500000, content: 'x'.repeat(500000) }])).toThrow(/500 KB/);
});
