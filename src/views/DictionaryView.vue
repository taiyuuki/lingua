<script setup lang="ts">
import DictionaryManager from "../components/DictionaryManager.vue";
import WordLookup from "../components/WordLookup.vue";
</script>

<template>
  <div class="view">
    <header class="view-header">
      <h1>词典</h1>
      <p class="view-subtitle">
        导入本地词典，查单词优先显示词典释义
      </p>
    </header>
    <WordLookup />
    <DictionaryManager />
  </div>
</template>
