import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Everything must resolve from the bundle: no CDN, no remote fonts, no runtime
// network access of any kind (invariant 5). `tools/check-offline.sh` enforces it.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // The port is fixed so the Tauri config's devUrl can name it, and
  // `VE_DEV_PORT` moves both together: a driver run can then start beside a
  // dev server somebody else is already using, rather than failing on a
  // port in use or killing what is holding it.
  server: { port: Number(process.env.VE_DEV_PORT ?? 5173), strictPort: true },
  build: {
    outDir: "dist",
    target: "es2022",
    sourcemap: true,
    // Fail the build rather than silently emitting a remote reference.
    rollupOptions: { external: [] },
  },
  // Node by default: these are pure-logic tests. A component test opts into a
  // DOM per file with `// @vitest-environment happy-dom` (`SpeedFilter.test.tsx`
  // is the pattern). happy-dom rather than jsdom: jsdom 27's CSS parser is a
  // CommonJS build that `require()`s an ES module, which only Node 20.19+ and
  // 22.12+ allow, and the machine this is developed on runs 21.
  test: { environment: "node", include: ["src/**/*.test.ts?(x)"] },
});
