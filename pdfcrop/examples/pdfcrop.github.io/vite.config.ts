import { defineConfig } from 'vite';
import wasm from 'vite-plugin-wasm';
import topLevelAwait from 'vite-plugin-top-level-await';

export default defineConfig({
  plugins: [
    wasm(),
    topLevelAwait()
  ],
  build: {
    target: 'esnext',
    outDir: 'dist',
    assetsDir: 'assets',
    rollupOptions: {
      output: {
        manualChunks: {
          'pdf.js': ['pdfjs-dist']
        }
      }
    }
  },
  server: {
    host: '127.0.0.1',
    port: 8080,
    strictPort: true,
    open: false,
    fs: {
      // Allow serving files from pkg directory
      allow: ['..']
    }
  },
  optimizeDeps: {
    exclude: ['pdfcrop-web']
  }
});
