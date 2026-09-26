<script setup lang="ts">
import DictionaryManager from "../components/DictionaryManager.vue";
</script>

<template>
  <div class="view">
    <header class="view-header">
      <h1>词典管理</h1>
      <p class="view-subtitle">
        导入本地词典，拖动调整查询命中顺序
      </p>
    </header>
    <DictionaryManager />
  </div>
</template>
