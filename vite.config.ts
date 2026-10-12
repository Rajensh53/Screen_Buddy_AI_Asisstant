import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],

  // Keep Rust compiler errors visible in the terminal.
  clearScreen: false,
  server: {
    // Tauri's devUrl expects this exact port.
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**", "**/spike/**"],
    },
  },
  build: {
    // One page per Tauri window. Paths are relative to the project root,
    // so window URLs are the same in dev and in dist/.
    rolldownOptions: {
      input: {
        mascot: "src/mascot/index.html",
        popup: "src/popup/index.html",
        settings: "src/settings/index.html",
      },
    },
  },
});
