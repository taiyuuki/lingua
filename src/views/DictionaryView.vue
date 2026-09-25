<script setup lang="ts">
import DictionaryManager from "../components/DictionaryManager.vue";
import WordLookup from "../components/WordLookup.vue";
</script>

<template>
  <div class="view">
    <header class="view-header">
      <h1>词典</h1>
      <p class="view-subtitle">
        本地词典：翻译单词时词典优先，命中直接出结果
      </p>
    </header>
    <WordLookup />
    <DictionaryManager />
  </div>
</template>
