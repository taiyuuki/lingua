<script setup lang="ts">
import VocabularyManager from "../components/VocabularyManager.vue";
</script>

<template>
  <div class="view">
    <header class="view-header">
      <h1>生词本</h1>
      <p class="view-subtitle">
        查过的生词自动记录；笔记 / 标签 / 掌握程度随你标注
      </p>
    </header>
    <VocabularyManager />
  </div>
</template>
