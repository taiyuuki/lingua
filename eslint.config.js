import pluginVue from "eslint-plugin-vue";
import { defineConfigWithVueTs, vueTsConfigs } from "@vue/eslint-config-typescript";

export default defineConfigWithVueTs(
  {
    ignores: ["dist/**", "node_modules/**", "src-tauri/**", "crates/**", "target/**"],
  },
  pluginVue.configs["flat/recommended"],
  vueTsConfigs.recommended,
  {
    rules: {
      // 单文件入口组件（App.vue）允许单词命名
      "vue/multi-word-component-names": "off",
    },
  },
);
