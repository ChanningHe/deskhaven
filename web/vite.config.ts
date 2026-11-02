import { fileURLToPath, URL } from 'node:url'

import tailwindcss from '@tailwindcss/vite'
import { tanstackRouter } from '@tanstack/router-plugin/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

// Backend (deskhaven) default HTTP listener during development.
const BACKEND_URL = 'http://127.0.0.1:21114'

export default defineConfig({
  plugins: [
    // Must run before the React plugin so generated route code is transformed.
    tanstackRouter({ target: 'react', autoCodeSplitting: true }),
    react(),
    tailwindcss(),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  server: {
    proxy: {
      '/api': { target: BACKEND_URL, changeOrigin: false },
      '/admin/api': { target: BACKEND_URL, changeOrigin: false },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: false,
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}', 'scripts/**/*.test.ts'],
    restoreMocks: true,
    // Well above Testing Library's asyncUtilTimeout (src/test/setup.ts), so a slow
    // findBy fails with its own message instead of a bare test timeout.
    testTimeout: 15_000,
  },
})
