import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { DocCollection, DocHit } from '../lib/types';
import { Documents } from './Documents';

const collection: DocCollection = {
  id: 'c1', name: 'Notes', root: 'C:/docs', fileCount: 12, chunkCount: 40,
  indexedAt: 1_700_000_000, error: null, createdAt: 1,
};

const apiMocks = vi.hoisted(() => ({
  list: vi.fn<() => Promise<DocCollection[]>>(async () => [collection]),
  add: vi.fn(),
  remove: vi.fn(async () => {}),
  reindex: vi.fn(async () => collection),
  search: vi.fn<() => Promise<DocHit[]>>(async () => [
    { collectionId: 'c1', collection: 'Notes', path: 'policy.md', lineStart: 12, lineEnd: 30, snippet: 'vacation days…' },
  ]),
  open: vi.fn(async () => 'C:/docs'),
}));

vi.mock('../lib/api', () => ({
  nativeAvailable: true,
  errorMessage: String,
  api: {
    documentsListCollections: apiMocks.list,
    documentsAddCollection: apiMocks.add,
    documentsRemoveCollection: apiMocks.remove,
    documentsReindex: apiMocks.reindex,
    documentsSearch: apiMocks.search,
  },
}));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: apiMocks.open }));

describe('document collections', () => {
  it('lists collections with counts and exposes remove/reindex', async () => {
    render(<Documents />);
    await screen.findByText('Notes');
    expect(screen.getByText('C:/docs')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Remove Notes' }));
    await waitFor(() => expect(apiMocks.remove).toHaveBeenCalledWith('c1'));
    fireEvent.click(screen.getByRole('button', { name: 'Reindex Notes' }));
    await waitFor(() => expect(apiMocks.reindex).toHaveBeenCalledWith('c1'));
  });

  it('adds the picked folder with the entered name', async () => {
    apiMocks.add.mockResolvedValue(collection);
    render(<Documents />);
    await screen.findByText('Notes');
    fireEvent.change(screen.getByLabelText('Collection name'), { target: { value: 'Manuals' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add folder' }));
    await waitFor(() => expect(apiMocks.add).toHaveBeenCalledWith('C:/docs', 'Manuals'));
  });

  it('searches and shows exact-line citations', async () => {
    render(<Documents />);
    await screen.findByText('Notes');
    fireEvent.change(screen.getByLabelText('Search documents'), { target: { value: 'vacation' } });
    fireEvent.click(screen.getByRole('button', { name: 'Search' }));
    await screen.findByText('policy.md:12–30');
    await waitFor(() => expect(apiMocks.search).toHaveBeenCalledWith('vacation', null));
  });
});
