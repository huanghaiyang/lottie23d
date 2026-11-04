import vue from '@vitejs/plugin-vue';
import { defineConfig } from 'vite';
import lottieLibrary from './scripts/library-plugin.mjs';

export default defineConfig({
  base: './',
  // 资源库插件：把上传的 Lottie 持久化到项目根目录的 data/（仅 dev / preview 生效）。
  plugins: [vue(), lottieLibrary()],
  server: {
    port: 5179,
  },
  build: {
    chunkSizeWarningLimit: 1600,
  },
});
