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
  NSpace,
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
        <n-space
          vertical
          :size="12"
        >
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

          <n-space
            v-if="importing && importProgress"
            vertical
            :size="4"
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
          </n-space>

          <n-space align="center">
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
          </n-space>
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
        </n-space>
      </n-form>
    </n-card>
  </div>
</template>

<style scoped>
.format-hint {
  display: block;
  font-size: 12px;
  white-space: pre-line;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}
</style>
