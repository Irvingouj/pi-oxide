import { defineConfig } from 'vite';

export default defineConfig({
  root: 'public',
  envDir: '../',
  envPrefix: 'VITE_',
  build: {
    outDir: '../dist',
  },
  server: {
    port: 5173,
    open: true,
  },
});
