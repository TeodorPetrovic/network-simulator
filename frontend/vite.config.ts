import react from "@vitejs/plugin-react";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    tanstackStart({
      spa: {
        enabled: true,
        prerender: { outputPath: "/index.html" },
      },
    }),
    tailwindcss(),
    react(),
  ],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  server: {
    host: "127.0.0.1",
    proxy: {
      "/v3": { target: "http://127.0.0.1:3080", changeOrigin: true, ws: true },
      "/healthz": { target: "http://127.0.0.1:3080", changeOrigin: true },
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/testSetup.ts"],
    restoreMocks: true,
    clearMocks: true,
  },
});
