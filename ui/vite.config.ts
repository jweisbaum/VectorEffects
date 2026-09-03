import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Everything must resolve from the bundle: no CDN, no remote fonts, no runtime
// network access of any kind (invariant 5). `tools/check-offline.sh` enforces it.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    outDir: "dist",
    target: "es2022",
    sourcemap: true,
    // Fail the build rather than silently emitting a remote reference.
    rollupOptions: { external: [] },
  },
  // Node by default: these are pure-logic tests. Component tests added in M5
  // opt into a DOM per file with `// @vitest-environment jsdom`.
  test: { environment: "node", include: ["src/**/*.test.ts?(x)"] },
});
