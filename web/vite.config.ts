import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  // viem + React land at ~510 kB minified (~150 kB gzip); fine for a dashboard.
  build: { chunkSizeWarningLimit: 700 },
});
