<script setup lang="ts">
import { onMounted } from "vue";
import { storeToRefs } from "pinia";
import { useAppStore } from "../stores/app";
import { useUpdaterStore } from "../stores/updater";

// 关于与更新：当前版本 + 手动检查 + 下载安装（完成后自动重启）。
// 启动时的静默检查在 App.vue 触发；这里负责展示与手动操作。
const appStore = useAppStore();
const { appInfo } = storeToRefs(appStore);

const updater = useUpdaterStore();
const { checking, downloading, progress, available, checkedAt, error } =
  storeToRefs(updater);

onMounted(() => {
  // 直达设置页时 appInfo 可能尚未加载。
  if (!appInfo.value) {
    void appStore.refresh();
  }
});
</script>

<template>
  <n-card
    title="关于与更新"
    size="small"
  >
    <n-descriptions
      :column="1"
      size="small"
      label-placement="left"
    >
      <n-descriptions-item label="当前版本">
        {{ appInfo ? `${appInfo.name} v${appInfo.version}` : "—" }}
      </n-descriptions-item>
    </n-descriptions>

    <!-- 有新版本：manifest 摘要 + 更新操作 -->
    <template v-if="available">
      <n-alert
        type="info"
        :show-icon="true"
        style="margin: 12px 0"
      >
        发现新版本 v{{ available.version }}，安装完成后将自动重启。
      </n-alert>
      <div class="action-row">
        <n-button
          type="primary"
          size="small"
          :loading="downloading"
          :disabled="checking"
          @click="updater.downloadAndInstall()"
        >
          立即更新
        </n-button>
        <n-button
          size="small"
          secondary
          :disabled="downloading"
          :loading="checking"
          @click="updater.check()"
        >
          重新检查
        </n-button>
      </div>
      <n-progress
        v-if="downloading || progress > 0"
        type="line"
        :percentage="progress"
        :indicator-placement="'inside'"
        processing
        style="margin-top: 12px"
      />
    </template>

    <!-- 无新版本：手动检查入口 -->
    <div
      v-else
      class="action-row"
      style="margin-top: 12px"
    >
      <n-button
        size="small"
        secondary
        :loading="checking"
        @click="updater.check()"
      >
        检查更新
      </n-button>
      <n-text
        v-if="checkedAt"
        depth="3"
      >
        已是最新（{{ checkedAt }} 检查）
      </n-text>
    </div>

    <n-alert
      v-if="error"
      type="error"
      :show-icon="true"
      style="margin-top: 12px"
    >
      {{ error }}
    </n-alert>
  </n-card>
</template>

<style scoped>
/* 与 RouterSettings 的同名类同款（各自 scoped）。 */
.action-row {
  display: flex;
  flex-flow: row wrap;
  align-items: center;
  gap: 8px 12px;
}
</style>
