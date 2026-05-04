import { defineConfig } from 'vite';
import { resolve } from 'path';

// Multi-page static site for openxgram.org
export default defineConfig({
  base: '/',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        index: resolve(__dirname, 'index.html'),
        why: resolve(__dirname, 'why/index.html'),
        features: resolve(__dirname, 'features/index.html'),
        quickstart: resolve(__dirname, 'quickstart/index.html'),
        guide: resolve(__dirname, 'guide/index.html'),
        usecases: resolve(__dirname, 'usecases/index.html'),
        architecture: resolve(__dirname, 'architecture/index.html'),
        docs: resolve(__dirname, 'docs/index.html'),
      },
    },
  },
});
