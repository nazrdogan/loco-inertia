import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig(({ command }) => ({
  plugins: [react()],
  // Built files are served by Loco under /static (see config/production.yaml).
  base: command === "build" ? "/static/" : "/",
  build: {
    manifest: true,
    outDir: "dist",
    rollupOptions: { input: "src/main.tsx" },
  },
  server: {
    port: 5173,
    strictPort: true,
    // Assets referenced from pages served by Loco (localhost:5150) must resolve to Vite.
    origin: "http://localhost:5173",
  },
}));
