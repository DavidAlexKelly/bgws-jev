// The editor's map bundle: dist/map.js and dist/map.css, embedded by
// bgws-server next to the viewer's app.js (see src/map-bundle.ts).
import { defineConfig } from "vite";

export default defineConfig({
  build: {
    outDir: "dist",
    emptyOutDir: false,
    cssCodeSplit: false,
    chunkSizeWarningLimit: 4000,
    rollupOptions: {
      input: "src/map-bundle.ts",
      output: {
        format: "iife",
        entryFileNames: "map.js",
        assetFileNames: (a) => (a.names?.[0] ?? a.name ?? "").endsWith(".css") ? "map.css" : "[name][extname]",
      },
    },
  },
});
