<script setup lang="ts">
import RouterSettings from "../components/RouterSettings.vue";
import UpdateChecker from "../components/UpdateChecker.vue";
</script>

<template>
  <div class="view">
    <header class="view-header">
      <h1>设置</h1>
      <p class="view-subtitle">
        主题 / 目标语言 / 快捷键 / 识别引擎
      </p>
    </header>
    <RouterSettings />
    <UpdateChecker />
  </div>
</template>
