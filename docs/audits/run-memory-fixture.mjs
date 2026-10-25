// Execute current TS guidance functions in a temporary directory.
// Run from the repository root: node docs/audits/run-memory-fixture.mjs
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import ts from 'typescript';

const repo = fileURLToPath(new URL('../../', import.meta.url));
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'locallm-memory-audit-'));
try {
  for (const name of ['types', 'modelStudio', 'runtimeGuidance']) {
    const source = fs.readFileSync(path.join(repo, `src/lib/${name}.ts`), 'utf8')
      .replace("from './types'", "from './types.mjs'");
    const result = ts.transpileModule(source, {
      compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
    });
    fs.writeFileSync(path.join(scratch, `${name}.mjs`), result.outputText);
  }
  const { recommendedConfig, estimateMemory } = await import(pathToFileURL(path.join(scratch, 'runtimeGuidance.mjs')));
  const { fitKind } = await import(pathToFileURL(path.join(scratch, 'modelStudio.mjs')));
  const model = {
    architecture: 'llama', contextLength: 131072, blockCount: 32,
    embeddingLength: 4096, headCount: 32, headCountKv: 8,
    keyLength: null, valueLength: null, fileBytes: 4 * 2 ** 30,
  };
  const hardware = {
    logicalCpus: 8, memoryTotalBytes: 8 * 2 ** 30,
    memoryAvailableBytes: 4 * 2 ** 30, gpus: [], gpuStatus: 'none', sampledAt: 0,
  };
  const recommended = recommendedConfig(model, hardware);
  console.log(JSON.stringify({
    scenario: '8 GiB total RAM, 4 GiB available, 4 GiB weights',
    recommendedContext: recommended.contextLength,
    recommendedGpuLayers: recommended.gpuLayers,
    estimatedRamGiB: estimateMemory(model, recommended).ramBytes / 2 ** 30,
  }, null, 2));
  console.log('4 GiB weights, 2 GiB GPU 0, 24 GiB GPU 1:', fitKind(4 * 2 ** 30, {
    ...hardware,
    gpus: [
      { memoryTotalMib: 2048, memoryUsedMib: 512 },
      { memoryTotalMib: 24576, memoryUsedMib: 512 },
    ],
  }));
} finally {
  fs.rmSync(scratch, { recursive: true, force: true });
}
