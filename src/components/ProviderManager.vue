<script setup lang="ts">
import { onMounted } from "vue";
import { storeToRefs } from "pinia";
import {
  NAlert,
  NButton,
  NCard,
  NEmpty,
  NForm,
  NFormItem,
  NInput,
  NList,
  NListItem,
  NPopconfirm,
  NSelect,
  NSpace,
  NTag,
  NText,
} from "naive-ui";
import { useProvidersStore } from "../stores/providers";

const store = useProvidersStore();
const {
  providers,
  loadError,
  saveError,
  saving,
  testing,
  testResults,
  form,
} = storeToRefs(store);

const timeoutOptions = [30, 60, 90, 120, 180, 300].map((seconds) => ({
  label: `${seconds} 秒`,
  value: seconds,
}));

onMounted(() => {
  void store.load();
});

function testResultText(id: string): string {
  const result = testResults.value[id];
  if (!result) {
    return "";
  }
  return result.ok
    ? `✓ 连接成功（${result.latencyMs}ms）${result.reply ?? ""}`
    : `✗ ${result.error ?? "unknown"}`;
}
</script>

<template>
  <n-card
    title="AI Providers"
    size="small"
  >
    <template #header-extra>
      <n-button
        v-if="!form.open"
        size="small"
        type="primary"
        @click="store.openCreate()"
      >
        添加
      </n-button>
    </template>

    <n-text
      depth="3"
      style="display: block; margin-bottom: 12px"
    >
      兼容 OpenAI 协议的服务均可接入（OpenAI / DeepSeek / 本地 Ollama、LM Studio
      等）。API Key 保存在系统钥匙串中，不会写入数据库。
    </n-text>
    <n-alert
      v-if="loadError"
      type="error"
      :show-icon="true"
      style="margin-bottom: 12px"
    >
      {{ loadError }}
    </n-alert>

    <!-- 列表 -->
    <n-list
      v-if="providers.length > 0 && !form.open"
      :show-divider="true"
    >
      <n-list-item
        v-for="provider in providers"
        :key="provider.id"
      >
        <div class="row">
          <div class="row-main">
            <n-space
              :size="6"
              align="center"
              style="margin-bottom: 2px"
            >
              <n-text strong>
                {{ provider.name }}
              </n-text>
              <n-tag
                size="small"
                :bordered="true"
              >
                {{ provider.id }}
              </n-tag>
              <n-tag
                v-if="provider.hasApiKey"
                size="small"
                type="success"
                :bordered="false"
              >
                已存密钥
              </n-tag>
            </n-space>
            <n-text
              depth="3"
              class="row-detail"
            >
              {{ provider.baseUrl }}<template v-if="provider.model">
                · {{ provider.model }}
              </template>
            </n-text>
            <n-text
              v-if="testResults[provider.id]"
              class="row-detail"
              :type="testResults[provider.id]!.ok ? 'success' : 'error'"
              style="display: block"
            >
              {{ testResultText(provider.id) }}
            </n-text>
          </div>
          <n-space :size="6">
            <n-button
              size="small"
              secondary
              :loading="testing[provider.id] === true"
              @click="store.runTest(provider.id)"
            >
              测试
            </n-button>
            <n-button
              size="small"
              secondary
              @click="store.openEdit(provider)"
            >
              编辑
            </n-button>
            <n-popconfirm
              :show-icon="false"
              @positive-click="store.remove(provider.id)"
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
              删除后该 Provider 的 API Key 也会从钥匙串清除，确定？
            </n-popconfirm>
          </n-space>
        </div>
      </n-list-item>
    </n-list>
    <n-empty
      v-else-if="!form.open"
      description="还没有 Provider，点击「添加」配置第一个（例如本地 Ollama：http://localhost:11434/v1）"
      style="margin: 12px 0"
    />

    <!-- 表单 -->
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
            label="ID（英文标识，保存后不可改）"
            style="flex: 1"
          >
            <n-input
              v-model:value="form.id"
              placeholder="deepseek / local-ollama"
              :disabled="!form.isNew"
              style="width: 100%"
            />
          </n-form-item>
          <n-form-item
            label="名称"
            style="flex: 1"
          >
            <n-input
              v-model:value="form.name"
              placeholder="DeepSeek"
            />
          </n-form-item>
        </n-space>

        <n-form-item label="接口地址（baseUrl）">
          <n-input
            v-model:value="form.baseUrl"
            placeholder="https://api.deepseek.com/v1 或 http://localhost:11434/v1"
          />
        </n-form-item>

        <n-space :size="12">
          <n-form-item
            label="模型"
            style="flex: 1"
          >
            <n-input
              v-model:value="form.model"
              placeholder="deepseek-chat / qwen2.5:7b …"
            />
          </n-form-item>
          <n-form-item
            label="API Key"
            style="flex: 1"
          >
            <n-input
              v-model:value="form.apiKeyInput"
              type="password"
              show-password-on="click"
              placeholder="留空表示不修改"
            />
          </n-form-item>
        </n-space>

        <n-space :size="12">
          <n-form-item
            label="超时"
            style="flex: 1"
          >
            <n-select
              v-model:value="form.timeoutSecs"
              :options="timeoutOptions"
            />
          </n-form-item>
          <n-form-item
            label="厂商扩展参数（JSON 对象，逐键合并进请求体）"
            style="flex: 2"
          >
            <n-input
              v-model:value="form.extraBody"
              type="textarea"
              :rows="2"
              placeholder="{&quot;thinking&quot;:{&quot;type&quot;:&quot;disabled&quot;}}（智谱 GLM 关闭深度思考，翻译类任务建议配置）"
            />
          </n-form-item>
        </n-space>

        <n-space>
          <n-button
            type="primary"
            :loading="saving"
            @click="store.save()"
          >
            保存
          </n-button>
          <n-button
            secondary
            @click="store.closeForm()"
          >
            取消
          </n-button>
        </n-space>
        <n-alert
          v-if="saveError"
          type="error"
          :show-icon="true"
        >
          {{ saveError }}
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
  display: block;
  font-size: 12.5px;
  overflow-wrap: anywhere;
}
</style>
