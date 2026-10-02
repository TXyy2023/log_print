import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
export default defineConfig({
  plugins: [vue()],
  build: {
    outDir: "../assets",
    emptyOutDir: true,
    chunkSizeWarningLimit: 1800,
    rolldownOptions: {
      output: {
        codeSplitting: {
          includeDependenciesRecursively: false,
          groups: [
            {
              name: "tables",
              test: /node_modules[\\/](ag-grid-community|ag-grid-vue3)[\\/]/,
            },
            { name: "charts", test: /node_modules[\\/](echarts|zrender)[\\/]/ },
            {
              name: "canvas",
              test: /node_modules[\\/](@vue-flow|gridstack)[\\/]/,
            },
            {
              name: "components",
              test: /node_modules[\\/](element-plus|@element-plus)[\\/]/,
            },
            { name: "vendor", test: /node_modules/ },
          ],
        },
      },
    },
  },
});
