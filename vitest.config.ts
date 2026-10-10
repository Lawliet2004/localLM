import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  test: {
    projects: [
      { extends: true, test: { name: 'web', environment: 'node', include: ['src/web/**/*.test.ts'] } },
      { extends: true, test: {
        name: 'ui', environment: 'jsdom', setupFiles: ['./src/test/setup.ts'],
        include: ['src/**/*.test.{ts,tsx}'], exclude: ['src/web/**'],
      } },
    ],
  },
});
