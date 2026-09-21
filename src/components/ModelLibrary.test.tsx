import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, expect, it, vi } from 'vitest';
import { ModelLibrary } from './ModelLibrary';
import type { HubSearchHit, InstalledModel } from '../lib/types';
import type { ModelInstallStatus } from '../lib/api';
const mock = vi.hoisted(() => ({
  searchHuggingFace: vi.fn(async (): Promise<HubSearchHit[]> => []),
  huggingFaceFiles: vi.fn(),
  downloadHuggingFaceModel: vi.fn(),
  deleteInstalledModel: vi.fn(),
  modelInstallStatus: vi.fn(async (): Promise<ModelInstallStatus> => ({ busy: false, phase: 'idle', received: 0, total: 0, path: null, error: null })),
  cancelModelInstall: vi.fn(),
  hardwareStatus: vi.fn(async () => ({ logicalCpus: 8, memoryTotalBytes: 16 * 2 ** 30, memoryAvailableBytes: 8 * 2 ** 30, gpuStatus: 'ok', sampledAt: 0, gpus: [{ name: 'GPU', uuid: 'g', memoryUsedMib: 0, memoryTotalMib: 4096, utilizationPercent: 0, driverVersion: '1' }] })),
}));
vi.mock('../lib/api', () => ({ api: mock, nativeAvailable: true, errorMessage: String }));
const model: InstalledModel = { id: 'C:/models/new.gguf', path: 'C:/models/new.gguf', filename: 'new.gguf', bytes: 1024, repo: 'publisher/new-GGUF', revision: 'a'.repeat(40), files: ['C:/models/new.gguf'], complete: true };
beforeEach(() => {
  vi.clearAllMocks();
  mock.searchHuggingFace.mockResolvedValue([]);
  mock.modelInstallStatus.mockResolvedValue({ busy: false, phase: 'idle', received: 0, total: 0, path: null, error: null });
});
const props = { models: [model], loading: false, busy: false, selectedPath: '', loadedPath: null, onRefresh: vi.fn(async () => {}), onUse: vi.fn(async () => {}), onDeleted: vi.fn(async () => {}) };
it('uses any downloaded model and requires an explicit delete action', async () => {
  render(<ModelLibrary {...props} />);
  fireEvent.click(screen.getByRole('tab', { name: /My Models/ }));
  fireEvent.click(screen.getByRole('button', { name: 'Use model' }));
  await waitFor(() => expect(props.onUse).toHaveBeenCalledWith(model));
  await waitFor(() => expect(screen.getByRole('button', { name: 'Delete new.gguf' })).toBeEnabled());
  fireEvent.click(screen.getByRole('button', { name: 'Delete new.gguf' }));
  expect(mock.deleteInstalledModel).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Delete from disk' }));
  await waitFor(() => expect(mock.deleteInstalledModel).toHaveBeenCalledWith(model.id));
  expect(props.onRefresh).toHaveBeenCalled();
});
it('resolves an arbitrary repository and downloads its selected immutable revision', async () => {
  mock.huggingFaceFiles.mockResolvedValue({ repo: model.repo, revision: model.revision, files: [{ filename: 'folder/new.gguf', bytes: 1024, sha256: 'b'.repeat(64) }] });
  mock.downloadHuggingFaceModel.mockResolvedValue({ busy: false, phase: 'ready', received: 1024, total: 1024, path: model.path, error: null });
  render(<ModelLibrary {...props} />);
  fireEvent.change(screen.getByLabelText('Search Hugging Face'), { target: { value: model.repo } });
  fireEvent.click(screen.getByRole('button', { name: 'Search' }));
  await screen.findByRole('button', { name: 'Download model' });
  fireEvent.click(screen.getByRole('button', { name: 'Download model' }));
  await waitFor(() => expect(mock.downloadHuggingFaceModel).toHaveBeenCalledWith(model.repo, model.revision, 'folder/new.gguf', undefined, null));
  expect(await screen.findByText('Download verified. Choose Use model in your library.')).toBeVisible();
});
it('downloads the matching vision projector with a multimodal model', async () => {
  mock.huggingFaceFiles.mockResolvedValue({
    repo: 'owner/arex-GGUF', revision: model.revision,
    files: [
      { filename: 'BAAI_AREX-Turbo-Q4_K_M.gguf', bytes: 1024, sha256: 'b'.repeat(64) },
      { filename: 'mmproj-BAAI_AREX-Turbo-Q4_K_M.gguf', bytes: 256, sha256: 'c'.repeat(64) },
    ],
  });
  mock.downloadHuggingFaceModel.mockResolvedValue({ busy: false, phase: 'ready', received: 1280, total: 1280, path: model.path, error: null });
  render(<ModelLibrary {...props} />);
  fireEvent.change(screen.getByLabelText('Search Hugging Face'), { target: { value: 'owner/arex-GGUF' } });
  fireEvent.click(screen.getByRole('button', { name: 'Search' }));
  await screen.findByLabelText('Vision projector');
  expect(screen.getByLabelText('Vision projector')).toHaveValue('mmproj-BAAI_AREX-Turbo-Q4_K_M.gguf');
  fireEvent.click(screen.getByRole('button', { name: 'Download model + projector' }));
  await waitFor(() => expect(mock.downloadHuggingFaceModel).toHaveBeenCalledWith('owner/arex-GGUF', model.revision, 'BAAI_AREX-Turbo-Q4_K_M.gguf', undefined, 'mmproj-BAAI_AREX-Turbo-Q4_K_M.gguf'));
  expect(await screen.findByText('Model and vision projector verified. Choose Use model in your library.')).toBeVisible();
});
it('keeps failed downloads retryable and does not make incomplete files usable', async () => {
  mock.huggingFaceFiles.mockRejectedValue(new Error('Repository unavailable'));
  render(<ModelLibrary {...props} models={[{ ...model, complete: false }]} />);
  fireEvent.click(screen.getByRole('tab', { name: /My Models/ }));
  expect(screen.getByRole('button', { name: 'Use model' })).toBeDisabled();
  fireEvent.click(screen.getByRole('tab', { name: 'Discover' }));
  fireEvent.change(screen.getByLabelText('Search Hugging Face'), { target: { value: 'publisher/new-GGUF' } });
  fireEvent.click(screen.getByRole('button', { name: 'Search' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('Repository unavailable');
  expect(screen.getByRole('button', { name: 'Search' })).toBeEnabled();
});
it('lists popular GGUF repositories on open', async () => {
  mock.searchHuggingFace.mockResolvedValue([{ id: 'bartowski/Qwen-GGUF', downloads: 42, likes: 3 }]);
  render(<ModelLibrary {...props} models={[]} />);
  expect(await screen.findByRole('button', { name: /bartowski\/Qwen-GGUF/ })).toBeVisible();
  expect(screen.getByText('Popular GGUF models')).toBeVisible();
  expect(screen.getByText('42 downloads')).toBeVisible();
  expect(mock.searchHuggingFace).toHaveBeenCalledWith('');
});
it('shows catalog models as Hugging Face entries with use and delete', () => {
  const catalog = { ...model, filename: 'MiniCPM5-2B.Q6_K.gguf', repo: 'prithivMLmods/MiniCPM5-2B-GGUF' };
  render(<ModelLibrary {...props} models={[catalog]} />);
  fireEvent.click(screen.getByRole('tab', { name: /My Models/ }));
  expect(screen.getByText('MiniCPM5-2B · Q6_K')).toBeVisible();
  expect(screen.getByText('Hugging Face · prithivMLmods/MiniCPM5-2B-GGUF')).toBeVisible();
  expect(screen.getByRole('button', { name: 'Use model' })).toBeEnabled();
  expect(screen.getByRole('button', { name: 'Delete MiniCPM5-2B · Q6_K' })).toBeEnabled();
});
it('opens a Hugging Face URL and shows whether the file fits this GPU', async () => {
  mock.huggingFaceFiles.mockResolvedValue({ repo: 'bartowski/Qwen-GGUF', revision: model.revision, files: [{ filename: 'Qwen-Q4_K_M.gguf', bytes: 2 * 2 ** 30, sha256: 'b'.repeat(64) }] });
  render(<ModelLibrary {...props} models={[]} />);
  fireEvent.change(screen.getByLabelText('Search Hugging Face'), { target: { value: 'https://huggingface.co/bartowski/Qwen-GGUF' } });
  fireEvent.click(screen.getByRole('button', { name: 'Search' }));
  expect(await screen.findByText('Fits GPU')).toBeVisible();
  expect(screen.getAllByText('Q4_K_M').length).toBeGreaterThan(0);
  expect(mock.huggingFaceFiles).toHaveBeenCalledWith('bartowski/Qwen-GGUF', undefined);
});
it('resumes an interrupted download from saved bytes', async () => {
  mock.downloadHuggingFaceModel.mockResolvedValue({ busy: false, phase: 'ready', received: 1024, total: 1024, path: model.path, error: null });
  render(<ModelLibrary {...props} models={[{ ...model, complete: false, received: 512 }]} />);
  fireEvent.click(screen.getByRole('tab', { name: /My Models/ }));
  expect(screen.getByText(/of .* saved/)).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: 'Resume download' }));
  await waitFor(() => expect(mock.downloadHuggingFaceModel).toHaveBeenCalledWith(model.repo, model.revision, model.filename, undefined, null));
});
it('resumes an ambiguous multimodal download with its persisted projector', async () => {
  mock.downloadHuggingFaceModel.mockResolvedValue({ busy: false, phase: 'ready', received: 1024, total: 1024, path: model.path, error: null });
  render(<ModelLibrary {...props} models={[{ ...model, complete: false, received: 512, projectorFilename: 'mmproj-model-f16.gguf' }]} />);
  fireEvent.click(screen.getByRole('tab', { name: /My Models/ }));
  expect(screen.getByText(/Vision: mmproj-model-f16\.gguf/)).toBeVisible();
  fireEvent.click(screen.getByRole('button', { name: 'Resume download' }));
  await waitFor(() => expect(mock.downloadHuggingFaceModel).toHaveBeenCalledWith(model.repo, model.revision, model.filename, undefined, 'mmproj-model-f16.gguf'));
});
it('resumes a legacy multimodal download directly as text-only', async () => {
  mock.downloadHuggingFaceModel.mockResolvedValue({ busy: false, phase: 'ready', received: 1024, total: 1024, path: model.path, error: null });
  render(<ModelLibrary {...props} models={[{ ...model, complete: false, received: 512, filename: 'Ternary-Bonsai-2-27B-PTQ1_0.gguf' }]} />);
  fireEvent.click(screen.getByRole('tab', { name: /My Models/ }));
  fireEvent.click(screen.getByRole('button', { name: 'Resume download' }));
  await waitFor(() => expect(mock.downloadHuggingFaceModel).toHaveBeenCalledWith(model.repo, model.revision, 'Ternary-Bonsai-2-27B-PTQ1_0.gguf', undefined, null));
  expect(mock.huggingFaceFiles).not.toHaveBeenCalled();
  expect(screen.getByRole('tab', { name: /My Models/ })).toHaveAttribute('aria-selected', 'true');
});
it('allows a text-only download when a repository has multiple projectors', async () => {
  mock.huggingFaceFiles.mockResolvedValue({
    repo: 'owner/multi-GGUF', revision: model.revision,
    files: [
      { filename: 'Ternary-Bonsai-2-27B-PTQ1_0.gguf', bytes: 1024, sha256: 'b'.repeat(64) },
      { filename: 'mmproj-F16.gguf', bytes: 64, sha256: 'c'.repeat(64) },
      { filename: 'mmproj-Q8_0.gguf', bytes: 96, sha256: 'd'.repeat(64) },
    ],
  });
  mock.downloadHuggingFaceModel.mockResolvedValue({ busy: false, phase: 'ready', received: 1120, total: 1120, path: model.path, error: null });
  render(<ModelLibrary {...props} models={[]} />);
  fireEvent.change(screen.getByLabelText('Search Hugging Face'), { target: { value: 'owner/multi-GGUF' } });
  fireEvent.click(screen.getByRole('button', { name: 'Search' }));
  await screen.findByLabelText('Vision projector');
  expect(screen.getByLabelText('Vision projector')).toHaveValue('');
  expect(screen.getByLabelText('Download PTQ1_0')).toBeEnabled();
  fireEvent.click(screen.getByLabelText('Download PTQ1_0'));
  await waitFor(() => expect(mock.downloadHuggingFaceModel).toHaveBeenCalledWith('owner/multi-GGUF', model.revision, 'Ternary-Bonsai-2-27B-PTQ1_0.gguf', undefined, null));
});
it('retries an incomplete download from saved Hugging Face provenance', async () => {
  mock.downloadHuggingFaceModel.mockResolvedValue({ busy: false, phase: 'ready', received: 1024, total: 1024, path: model.path, error: null });
  render(<ModelLibrary {...props} models={[{ ...model, complete: false }]} />);
  fireEvent.click(screen.getByRole('tab', { name: /My Models/ }));
  fireEvent.click(screen.getByRole('button', { name: 'Retry download' }));
  await waitFor(() => expect(mock.downloadHuggingFaceModel).toHaveBeenCalledWith(model.repo, model.revision, model.filename, undefined, null));
});

it('can use a complete model while a resumed download is still pending', async () => {
  mock.downloadHuggingFaceModel.mockImplementation(() => new Promise(() => {}));
  render(<ModelLibrary {...props} models={[model, { ...model, id: 'partial', filename: 'partial.gguf', complete: false, received: 512 }]} />);
  fireEvent.click(screen.getByRole('tab', { name: /My Models/ }));
  fireEvent.click(screen.getByRole('button', { name: 'Resume download' }));
  await waitFor(() => expect(mock.downloadHuggingFaceModel).toHaveBeenCalled());
  const use = screen.getAllByRole('button', { name: 'Use model' })[0];
  expect(use).toBeEnabled();
  fireEvent.click(use);
  await waitFor(() => expect(props.onUse).toHaveBeenCalledWith(model));
});

it('shows live bytes for an active partial file and keeps completed models usable', async () => {
  const partial = { ...model, id: 'partial', filename: 'partial.gguf', path: 'C:/models/partial.gguf.part', complete: false, received: 128 };
  mock.modelInstallStatus.mockResolvedValue({ busy: true, phase: 'resuming', received: 768, total: 1024, path: 'C:/models/partial.gguf', error: null });
  render(<ModelLibrary {...props} models={[model, partial]} />);
  fireEvent.click(screen.getByRole('tab', { name: /My Models/ }));
  expect(await screen.findByText('Downloading')).toBeVisible();
  expect(screen.queryByText('Interrupted')).not.toBeInTheDocument();
  expect(screen.getByRole('progressbar')).toHaveAttribute('value', '768');
  expect(screen.getAllByRole('button', { name: 'Use model' })[0]).toBeEnabled();
  expect(screen.getAllByRole('button', { name: 'Use model' })[1]).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Downloading…' })).toBeDisabled();
});
