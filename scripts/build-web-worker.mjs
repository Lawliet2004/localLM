import { build } from 'vite';

await build({
  configFile: false,
  logLevel: 'warn',
  ssr: { noExternal: ['fflate'] },
  build: {
    ssr: 'scripts/web-worker.mjs',
    outDir: 'src-tauri/resources/web',
    emptyOutDir: false,
    minify: false,
    rolldownOptions: { output: { entryFileNames: 'worker.mjs', codeSplitting: false } },
  },
});
