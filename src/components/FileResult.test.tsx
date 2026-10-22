import { fireEvent, render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { FileResult } from './FileResult';
it('defers file code and raw serialization until their disclosures open', () => {
  const raw = { toJSON: () => { throw new Error('raw accessed too early'); } };
  render(<FileResult path="src/very-long-file.ts" content="const value = 1;" raw={raw} />);
  expect(screen.queryByLabelText('typescript source')).toBeNull();
  fireEvent.click(document.querySelector('.file-result > summary')!);
  const code = screen.getByLabelText('typescript source');
  expect(code).toHaveTextContent('const value = 1;');
  fireEvent.click(document.querySelector('.file-result > summary')!);
  expect(screen.getByLabelText('typescript source')).toBe(code);
});
