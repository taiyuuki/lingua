<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useRouter } from "vue-router";
import { storeToRefs } from "pinia";
import {
  NAlert,
  NButton,
  NCard,
  NForm,
  NFormItem,
  NInput,
  NInputGroup,
  NProgress,
  NSelect,
  NText,
} from "naive-ui";
import {
  MDX_FORMAT_HINT,
  TSV_FORMAT_HINT,
  isMdxPath,
  useDictionariesStore,
} from "../stores/dictionaries";
import { SUPPORTED_LANGUAGES } from "../stores/settings";
import { pickDictionaryFilePath } from "../lib/api";

/** 独立导入页：每次进入都重新挂载，表单与返回按钮恒为单实例。 */
const store = useDictionariesStore();
const {
  form,
  importing,
  cancelling,
  importProgress,
  importError,
  importMessage,
  importWarning,
} = storeToRefs(store);
const router = useRouter();

// 进入导入页：空闲则重置表单与提示；后台导入中则保留现场继续显示进度。
onMounted(() => {
  store.beginImport();
});

/** 返回词典管理页；导入中返回则导入转后台继续（列表页有进度横条与取消入口）。 */
function backToList(): void {
  void router.push({ name: "dictionaries" });
}

const languageSelectOptions = computed(() =>
  SUPPORTED_LANGUAGES.map((lang) => ({
    label: `${lang.label}（${lang.code}）`,
    value: lang.code,
  })),
);

/** 选中的是 MDX 词库时切换格式提示与导入命令。 */
const isMdxFile = computed(() => isMdxPath(form.value.path));
const formatHint = computed(() => (isMdxFile.value ? MDX_FORMAT_HINT : TSV_FORMAT_HINT));
const importPercentage = computed(() => {
  const progress = importProgress.value;
  if (!progress || progress.total === 0) {
    return 0;
  }
  return Math.round((progress.imported / progress.total) * 100);
});

/** 文件选择框选词典文件：填路径，名称为空/仍是自动值时用文件名填入。 */
async function browseDictionaryFile(): Promise<void> {
  try {
    const path = await pickDictionaryFilePath();
    if (path) {
      store.setPickedPath(path);
    }
  } catch {
    // 选择器不可用时静默：手输路径的旧路径仍然可用。
  }
}
</script>

<template>
  <div class="view">
    <header class="view-header">
      <h1>导入词典</h1>
      <p class="view-subtitle">
        导入本地词典后，查单词优先显示词典释义；没有收录的词再由 AI 翻译
      </p>
    </header>
    <n-card size="small">
      <n-form
        label-placement="top"
        size="small"
        :show-feedback="false"
      >
        <!-- naive-ui 的 n-space 会在 v-if 子项反复显隐时重复渲染（包裹层 key 重复），
             导入页的按钮/进度/提示每次导入都会切换，必须用普通 flex 容器布局。 -->
        <div class="form-stack">
          <n-form-item label="名称">
            <n-input
              v-model:value="form.name"
              placeholder="选文件后自动填入，可手动修改"
            />
          </n-form-item>

          <div class="form-grid">
            <n-form-item label="词头语言">
              <n-select
                v-model:value="form.sourceLanguage"
                :options="languageSelectOptions"
              />
            </n-form-item>
            <n-form-item label="释义语言">
              <n-select
                v-model:value="form.glossLanguage"
                :options="languageSelectOptions"
              />
            </n-form-item>
          </div>

          <n-form-item label="词典文件路径">
            <n-input-group>
              <n-input
                v-model:value="form.path"
                placeholder="点「浏览…」选择 .tsv / .mdx，或输入文件路径"
              />
              <n-button
                secondary
                @click="browseDictionaryFile"
              >
                浏览…
              </n-button>
            </n-input-group>
          </n-form-item>
          <n-text
            depth="3"
            class="format-hint"
          >
            {{ formatHint }}
          </n-text>

          <div
            v-if="importing && importProgress"
            class="progress-stack"
          >
            <n-text depth="3">
              {{ importProgress.phase === "parsing"
                ? "正在解析词库索引…"
                : `正在导入：${importProgress.imported} / ${importProgress.total} 词条` }}
            </n-text>
            <n-progress
              type="line"
              :percentage="importPercentage"
              :height="8"
              :show-indicator="false"
            />
          </div>

          <div class="action-row">
            <n-button
              v-if="!importing"
              type="primary"
              @click="store.importDictionary()"
            >
              开始导入
            </n-button>
            <!-- 真取消：后端停止导入并清理半成品 -->
            <n-button
              v-if="importing"
              type="error"
              secondary
              :loading="cancelling"
              @click="store.cancelImport()"
            >
              取消导入
            </n-button>
            <!-- 唯一返回入口：路由跳转回词典管理页，不会随导入次数累积 -->
            <n-button
              secondary
              @click="backToList"
            >
              {{ importing ? "后台继续，返回列表" : "返回列表" }}
            </n-button>
            <n-text
              v-if="importMessage"
              type="success"
            >
              {{ importMessage }}
            </n-text>
          </div>
          <n-alert
            v-if="importWarning"
            type="warning"
            :show-icon="true"
          >
            {{ importWarning }}
          </n-alert>
          <n-alert
            v-if="importError"
            type="error"
            :show-icon="true"
          >
            {{ importError }}
          </n-alert>
        </div>
      </n-form>
    </n-card>
  </div>
</template>

<style scoped>
/* 三个容器是原 n-space 的等价 flex 布局（gap 对齐原渲染值）。 */
.form-stack {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.progress-stack {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.action-row {
  display: flex;
  flex-flow: row wrap;
  align-items: center;
  gap: 8px 12px;
}

.format-hint {
  display: block;
  font-size: 12px;
  white-space: pre-line;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}
</style>
