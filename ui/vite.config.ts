import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  resolve: {
    // 必须与 tsconfig.json 的 paths 同时存在。只配一处的话，一处能跑、
    // 另一处报模块找不到 —— 而且报错信息指向的是别的文件，很难顺藤摸瓜。
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  server: { port: 5173, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true },
});
