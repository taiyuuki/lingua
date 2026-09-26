<script setup lang="ts">
import { useRouter } from "vue-router";
import { NButton } from "naive-ui";
import WordLookup from "../components/WordLookup.vue";

/** 词典查词页；词典的导入与排序在「词典管理」页。 */
const router = useRouter();
</script>

<template>
  <div class="view">
    <header class="view-header view-header-row">
      <div>
        <h1>词典</h1>
        <p class="view-subtitle">
          查单词：内置词典优先，AI 翻译兜底
        </p>
      </div>
      <n-button
        secondary
        @click="router.push({ name: 'dictionaries' })"
      >
        管理词典
      </n-button>
    </header>
    <WordLookup />
  </div>
</template>

<style scoped>
.view-header-row {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: 12px;
}
</style>
