<script setup lang="ts">
import HistoryManager from "../components/HistoryManager.vue";
</script>

<template>
  <div class="view">
    <header class="view-header">
      <h1>历史</h1>
      <p class="view-subtitle">
        划词与截图翻译记录（自动保留最近 5000 条）
      </p>
    </header>
    <HistoryManager />
  </div>
</template>
