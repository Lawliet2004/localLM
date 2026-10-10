import { build } from 'vite';
import { copyFile, mkdir, mkdtemp, readFile, rm } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

// Copy only when the bytes changed, so a no-change rebuild does not bump the
// file mtime and make a live Tauri app recompile mid-test.
export async function publishIfChanged(source, destination) {
  const next = await readFile(source);
  if (existsSync(destination) && (await readFile(destination)).equals(next)) {
    return 'unchanged';
  }
  await mkdir(join(destination, '..'), { recursive: true });
  await copyFile(source, destination);
  return 'written';
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const outDir = await mkdtemp(join(tmpdir(), 'locallm-web-worker-'));
  try {
    await build({
    configFile: false,
    logLevel: 'warn',
    // The worker is extracted to the app-data dir and run standalone — it has no
    // node_modules next to it, so npm deps must be inlined into the bundle.
    // ('playwright' stays external: web-render.mjs imports it lazily for the
    // js_render fallback and it cannot be bundled.)
    ssr: { noExternal: ['fflate'] },
    build: {
      ssr: 'scripts/web-worker.mjs',
      outDir,
      emptyOutDir: true,
      minify: false,
      rolldownOptions: { output: { entryFileNames: 'worker.mjs', codeSplitting: false } },
    },
    });
    const destination = 'src-tauri/resources/web/worker.mjs';
    const outcome = await publishIfChanged(join(outDir, 'worker.mjs'), destination);
    console.log(`web worker ${outcome === 'unchanged' ? 'unchanged' : `written to ${destination}`}`);
  } finally { await rm(outDir, { recursive: true, force: true }); }
}
