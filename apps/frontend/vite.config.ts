import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import hotFile from '@sandpost/vite-plugin-hot-file'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [
    vue(),
    tailwindcss(),
    hotFile(),
  ],
  resolve: {
    alias: {
      '@': new URL('./src', import.meta.url).pathname
    }
  },
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': 'http://127.0.0.1:8025'
    },
  },
})
