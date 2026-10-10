import { expect, it, vi } from 'vitest';
import { api } from './api';

const invoke = vi.hoisted(() => vi.fn(async () => []));
vi.mock('@tauri-apps/api/core', () => ({ invoke, isTauri: () => true, Channel: class {} }));

it('loads skill dependencies through the registered native command', async () => {
  await api.skillDependencies('notebook');
  expect(invoke).toHaveBeenCalledWith('skill_dependencies', { id: 'notebook' });
});
