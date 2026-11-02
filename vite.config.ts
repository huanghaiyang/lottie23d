import vue from '@vitejs/plugin-vue';
import { defineConfig } from 'vite';

export default defineConfig({
  base: './',
  plugins: [vue()],
  server: {
    port: 5179,
  },
  build: {
    chunkSizeWarningLimit: 1600,
  },
});
