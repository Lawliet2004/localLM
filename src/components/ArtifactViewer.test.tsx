import { expect, it } from 'vitest';
import { parseDelimited } from './ArtifactViewer';
it('parses quoted commas, escaped quotes and multiline cells for table preview', () => {
  expect(parseDelimited('Name,Note\n"Alpha, Inc","said ""hello""\non two lines"\n')).toEqual([['Name', 'Note'], ['Alpha, Inc', 'said "hello"\non two lines']]);
});
