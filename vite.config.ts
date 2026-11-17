import { defineConfig } from "vite";

export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**", "**/native-bridge/**"],
    },
  },
  build: {
    outDir: "dist",
    target: ["es2021", "safari15"],
  },
});
