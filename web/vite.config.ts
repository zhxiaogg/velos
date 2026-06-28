import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// In production the apiserver serves this bundle, so the API is same-origin.
// In dev, Vite serves it and proxies the API/auth paths to the apiserver purely
// to avoid CORS — no auth is injected here; the browser obtains and sends its
// own credential (see src/auth.ts).
const VELOS = process.env.VELOS_SERVER ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // The apiserver embeds this directory (crates/apiserver/ui) and serves it.
  build: {
    outDir: "../crates/apiserver/ui",
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    proxy: {
      "/api": { target: VELOS, changeOrigin: true },
      "/auth": { target: VELOS, changeOrigin: true },
    },
  },
});
