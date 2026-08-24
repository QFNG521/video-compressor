import { defineConfig } from 'vite'

export default defineConfig({
  // Tauri 用 file:// 协议加载打包资源，必须相对路径
  base: './',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2021',
  },
  server: {
    port: 5173,
    strictPort: true,
  },
})
