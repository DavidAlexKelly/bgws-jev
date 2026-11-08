// The viewer's build: one script and one stylesheet with fixed names, which
// bgws-server embeds (apps/server/src/lib.rs). Everything is bundled, so the
// page needs the network only for map tiles.
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

export default defineConfig({
  plugins: [react()],
  // milsymbol's ES entry, past its exports map, so only the 2525C letter set is bundled (src/symbols.ts).
  resolve: { alias: { "milsymbol-esm": fileURLToPath(new URL("node_modules/milsymbol/index.esm.js", import.meta.url)) } },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    cssCodeSplit: false,
    chunkSizeWarningLimit: 4000,
    rollupOptions: {
      input: "index.html",
      output: {
        format: "iife",
        entryFileNames: "app.js",
        assetFileNames: (a) => (a.names?.[0] ?? a.name ?? "").endsWith(".css") ? "app.css" : "[name][extname]",
      },
    },
  },
  server: { proxy: { "/ws": { target: "ws://127.0.0.1:8765", ws: true } } },
});
