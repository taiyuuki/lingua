<script setup lang="ts">
import { onMounted, ref } from "vue";
import { storeToRefs } from "pinia";
import { useAppStore } from "../stores/app";
import { useSettingsStore } from "../stores/settings";
import { screenRecordingStatus, startOcrCapture } from "../lib/api";

const appStore = useAppStore();
const { appInfo, database, errorMessage } = storeToRefs(appStore);

const settings = useSettingsStore();
const { shortcut, permissionGranted } = storeToRefs(settings);

const screenRecording = ref<boolean | null>(null);
const ocrError = ref<string | null>(null);

async function refreshScreenRecording(): Promise<void> {
  try {
    screenRecording.value = await screenRecordingStatus();
  } catch {
    screenRecording.value = null;
  }
}

async function startOcr(): Promise<void> {
  ocrError.value = null;
  try {
    await startOcrCapture();
  } catch (error) {
    ocrError.value = String(error);
  }
  void refreshScreenRecording();
}

onMounted(() => {
  void appStore.refresh();
  void refreshScreenRecording();
});
</script>

<template>
  <div class="view">
    <header class="view-header">
      <h1>首页</h1>
      <p class="view-subtitle">
        划词翻译 · 截图 OCR · 生词本
      </p>
    </header>

    <n-card
      title="使用方法"
      size="small"
    >
      <p class="view-note">
        在任意应用中选中文本，按全局快捷键
        <n-tag
          size="small"
          :bordered="true"
        >
          {{ shortcut || "alt+t" }}
        </n-tag>
        ，翻译结果会在屏幕中央的 Popup 中显示；按
        <n-tag
          size="small"
          :bordered="true"
        >
          Esc
        </n-tag>
        或点击窗口外部关闭。
      </p>
      <n-alert
        v-if="permissionGranted === false"
        type="warning"
        :show-icon="true"
      >
        缺少辅助功能权限：请在「系统设置 → 隐私与安全性 → 辅助功能」中授权
        Lingua，否则无法读取选中文本。
      </n-alert>
      <n-alert
        v-if="errorMessage"
        type="error"
        :show-icon="true"
      >
        {{ errorMessage }}
      </n-alert>
    </n-card>

    <n-card
      title="截图翻译"
      size="small"
    >
      <p class="view-note">
        框选屏幕任意区域，识别出的文字自动走 AI
        翻译并在 Popup 中显示。OCR 引擎在「设置」页配置；截图不落盘。
      </p>
      <n-button
        type="primary"
        @click="startOcr"
      >
        截图翻译（框选屏幕区域）
      </n-button>
      <n-alert
        v-if="ocrError"
        type="error"
        :show-icon="true"
        style="margin-top: 10px"
      >
        {{ ocrError }}
      </n-alert>
    </n-card>

    <n-card
      title="运行状态"
      size="small"
    >
      <n-descriptions
        :column="1"
        size="small"
        label-placement="left"
      >
        <n-descriptions-item label="应用">
          {{ appInfo ? `${appInfo.name} v${appInfo.version}（${appInfo.os}/${appInfo.arch}）` : "—" }}
        </n-descriptions-item>
        <n-descriptions-item label="数据库">
          {{ database ? `schema v${database.schemaVersion} · ${database.migrationsTotal} 个 migration` : "—" }}
        </n-descriptions-item>
      </n-descriptions>
    </n-card>
  </div>
</template>
