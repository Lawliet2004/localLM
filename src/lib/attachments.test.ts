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

it('keeps unnamed clipboard images and binary uploads in the attachment envelope', async () => {
  const png = new Uint8Array(300000);
  png.set([137, 80, 78, 71]);
  const files = [
    new File([png], 'clipboard', { type: 'image/png' }),
    new File(['%PDF-1.7'], 'document.pdf'),
    new File([new Uint8Array([0, 0, 0, 16]), 'ftypisom'], 'clip.mp4'),
  ];
  const message = JSON.parse(composeMessage('Review', await Promise.all(files.map(readAttachment))));
  expect(message.attachments[0].imageUrl).toMatch(/^data:image\/png;base64,/);
  expect(message.attachments[1].mediaUrl).toMatch(/^data:application\/pdf;base64,/);
  expect(message.attachments[2].mediaUrl).toMatch(/^data:video\/mp4;base64,/);
});
