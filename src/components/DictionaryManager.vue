<script setup lang="ts">
import { computed, onMounted } from "vue";
import { storeToRefs } from "pinia";
import {
  NAlert,
  NButton,
  NCard,
  NEmpty,
  NForm,
  NFormItem,
  NInput,
  NInputGroup,
  NList,
  NListItem,
  NPopconfirm,
  NSelect,
  NSpace,
  NTag,
  NText,
} from "naive-ui";
import { TSV_FORMAT_HINT, useDictionariesStore } from "../stores/dictionaries";
import { pickTsvFilePath } from "../lib/api";

const store = useDictionariesStore();
const {
  dictionaries,
  loadError,
  importError,
  importMessage,
  importing,
  form,
  languageOptions,
} = storeToRefs(store);

const languageSelectOptions = computed(() =>
  languageOptions.value.map((lang) => ({
    label: `${lang.label}（${lang.code}）`,
    value: lang.code,
  })),
);

/** 文件选择框选 TSV，直接填进路径输入（仍可手改）。 */
async function browseTsv(): Promise<void> {
  try {
    const path = await pickTsvFilePath();
    if (path) {
      form.value.path = path;
    }
  } catch {
    // 选择器不可用时静默：手输路径的旧路径仍然可用。
  }
}

onMounted(() => {
  void store.load();
});

</script>

<template>
  <n-card
    title="词典"
    size="small"
  >
    <template #header-extra>
      <n-button
        v-if="!form.open"
        size="small"
        type="primary"
        @click="store.openImport()"
      >
        导入词典
      </n-button>
    </template>

    <n-text
      depth="3"
      style="display: block; margin-bottom: 12px"
    >
      本地词典优先：翻译单词时命中词典则直接出结果（快、离线、零成本），未命中才走
      AI。
    </n-text>
    <n-alert
      v-if="loadError"
      type="error"
      :show-icon="true"
      style="margin-bottom: 12px"
    >
      {{ loadError }}
    </n-alert>

    <!-- 词典列表 -->
    <n-list
      v-if="dictionaries.length > 0 && !form.open"
      :show-divider="true"
    >
      <n-list-item
        v-for="dictionary in dictionaries"
        :key="dictionary.id"
      >
        <div class="row">
          <div class="row-main">
            <n-space
              :size="6"
              align="center"
              style="margin-bottom: 2px"
            >
              <n-text strong>
                {{ dictionary.name }}
              </n-text>
              <n-tag
                size="small"
                :bordered="true"
              >
                {{ dictionary.id }}
              </n-tag>
            </n-space>
            <n-text
              depth="3"
              class="row-detail"
            >
              {{ dictionary.sourceLanguage }} → {{ dictionary.glossLanguage }} ·
              {{ dictionary.entryCount }} 个词条
            </n-text>
          </div>
          <n-popconfirm
            :show-icon="false"
            @positive-click="store.remove(dictionary.id)"
          >
            <template #trigger>
              <n-button
                size="small"
                type="error"
                secondary
              >
                删除
              </n-button>
            </template>
            全部词条数据将被移除，确定？
          </n-popconfirm>
        </div>
      </n-list-item>
    </n-list>
    <n-empty
      v-else-if="!form.open"
      description="还没有导入词典。可以把 CC-CEDICT / ECDICT 等公开词典转成 TSV 后导入。"
      style="margin: 12px 0"
    />

    <!-- 导入表单 -->
    <n-form
      v-if="form.open"
      label-placement="top"
      size="small"
      :show-feedback="false"
    >
      <n-space
        vertical
        :size="12"
      >
        <n-space :size="12">
          <n-form-item
            label="ID（英文标识）"
            style="flex: 1"
          >
            <n-input
              v-model:value="form.id"
              placeholder="my-en-zh"
            />
          </n-form-item>
          <n-form-item
            label="名称"
            style="flex: 1"
          >
            <n-input
              v-model:value="form.name"
              placeholder="我的英汉词典"
            />
          </n-form-item>
        </n-space>

        <n-space :size="12">
          <n-form-item
            label="词头语言"
            style="flex: 1"
          >
            <n-select
              v-model:value="form.sourceLanguage"
              :options="languageSelectOptions"
            />
          </n-form-item>
          <n-form-item
            label="释义语言"
            style="flex: 1"
          >
            <n-select
              v-model:value="form.glossLanguage"
              :options="languageSelectOptions"
            />
          </n-form-item>
        </n-space>

        <n-form-item label="TSV 文件路径">
          <n-input-group>
            <n-input
              v-model:value="form.path"
              placeholder="点「浏览…」选择，或输入 .tsv 文件路径"
            />
            <n-button
              secondary
              @click="browseTsv"
            >
              浏览…
            </n-button>
          </n-input-group>
        </n-form-item>
        <n-text
          depth="3"
          class="format-hint"
        >
          {{ TSV_FORMAT_HINT }}
        </n-text>

        <n-space align="center">
          <n-button
            type="primary"
            :loading="importing"
            @click="store.importTsv()"
          >
            {{ importing ? "导入中…" : "开始导入" }}
          </n-button>
          <n-button
            secondary
            @click="store.closeForm()"
          >
            取消
          </n-button>
          <n-text
            v-if="importMessage"
            type="success"
          >
            {{ importMessage }}
          </n-text>
        </n-space>
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
</template>

<style scoped>
.row {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
}

.row-main {
  min-width: 0;
}

.row-detail {
  font-size: 12.5px;
}

.format-hint {
  display: block;
  font-size: 12px;
  white-space: pre-line;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
}
</style>
