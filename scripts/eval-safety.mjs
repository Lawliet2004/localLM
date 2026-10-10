import { cpSync, lstatSync, mkdirSync } from 'node:fs';
import { isAbsolute, relative, resolve, sep } from 'node:path';

const excludedDirectoryNames = new Set([
  '.git', '.commandcode', '.local', '.vitest', '.codex', '.agents', '.claude',
  '.ssh', '.aws', '.azure', '.gnupg', 'node_modules', 'target',
  'dist', 'eval-results', 'graphify-out', 'coverage', 'playwright-report',
  'test-results',
]);
const excludedFileNames = new Set(['.npmrc', '.pypirc', '.netrc', 'credentials.json']);

export function copyRepoSnapshot(source, destination) {
  const sourcePath = resolve(source);
  const destinationPath = resolve(destination);
  const contains = (parent, child) => {
    const path = relative(parent, child);
    return !path || (!isAbsolute(path) && path !== '..' && !path.startsWith(`..${sep}`));
  };
  if (contains(sourcePath, destinationPath) || contains(destinationPath, sourcePath)) {
    throw new Error('The evaluation snapshot must be outside the source repository.');
  }

  cpSync(sourcePath, destinationPath, {
    recursive: true,
    filter: sourceEntry => {
      const path = relative(sourcePath, sourceEntry);
      if (!path) return true;
      const parts = path.split(sep);
      if (parts.some(part => excludedDirectoryNames.has(part))) return false;
      if (excludedFileNames.has(parts.at(-1).toLowerCase())) return false;
      if (parts.some(part => part === '.env' || part.startsWith('.env.'))) return false;
      if (/\.(?:sqlite3?|db|pem|key|p12)(?:-(?:wal|shm))?$/i.test(parts.at(-1))) return false;
      try { return !lstatSync(sourceEntry).isSymbolicLink(); }
      catch { return false; }
    },
  });
  return destinationPath;
}

export function createEvalRunDirectory(outputRoot, modelSlug, runId) {
  const segment = /^[a-z0-9][a-z0-9-]*$/i;
  if (!segment.test(modelSlug) || !segment.test(runId)) {
    throw new Error('Evaluation output names must be simple path segments.');
  }
  const parent = resolve(outputRoot, modelSlug);
  mkdirSync(parent, { recursive: true });
  const directory = resolve(parent, runId);
  mkdirSync(directory);
  return directory;
}

export function workspaceForTest(test, scratch, repoSnapshot) {
  if (test.workspace === 'repo') {
    if (!repoSnapshot) throw new Error(`Test ${test.id} requires an isolated repository snapshot.`);
    return resolve(repoSnapshot);
  }
  return resolve(scratch, `${String(test.id).padStart(2, '0')}-${test.slug}`);
}

export function requireReadyEvaluationModel(status, model) {
  const activePath = status.modelPath?.toLowerCase();
  if (status.phase !== 'ready' || !activePath || !activePath.includes(model.filename.toLowerCase())) {
    throw new Error(
      `Evaluation requires ${model.filename} to already be loaded; runtime is ${status.phase}. ` +
      'The evaluator will not switch models or write saved runtime settings.',
    );
  }
  return model;
}

export async function restoreWorkspace(invoke, savedWorkspacePath) {
  return invoke('set_workspace', { path: savedWorkspacePath ?? '' });
}

export async function withWorkspace(invoke, workspace, run, savedWorkspacePath) {
  const previousPath = savedWorkspacePath ?? (await invoke('get_workspace')).path ?? '';
  try {
    await invoke('set_workspace', { path: workspace });
    return await run();
  } finally {
    await restoreWorkspace(invoke, previousPath);
  }
}
