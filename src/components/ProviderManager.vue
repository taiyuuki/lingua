<script setup lang="ts">
import { onMounted } from "vue";
import { storeToRefs } from "pinia";
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

onMounted(() => {
  void store.load();
});
</script>

<template>
  <section class="card">
    <div class="card-header-row">
      <h2>AI Providers</h2>
      <button
        v-if="!form.open"
        class="btn btn-small"
        type="button"
        @click="store.openCreate()"
      >
        添加
      </button>
    </div>
    <p class="muted">
      兼容 OpenAI 协议的服务均可接入（OpenAI / DeepSeek / 本地 Ollama、LM Studio
      等）。API Key 保存在系统钥匙串中，不会写入数据库。
    </p>
    <p
      v-if="loadError"
      class="error-banner"
    >
      {{ loadError }}
    </p>

    <!-- 列表 -->
    <ul
      v-if="providers.length > 0 && !form.open"
      class="provider-list"
    >
      <li
        v-for="provider in providers"
        :key="provider.id"
        class="provider-row"
      >
        <div class="provider-info">
          <div class="provider-title">
            <strong>{{ provider.name }}</strong>
            <code class="provider-id">{{ provider.id }}</code>
          </div>
          <p class="provider-detail">
            {{ provider.baseUrl }}
            <template v-if="provider.model">
              · {{ provider.model }}
            </template>
            <template v-if="provider.hasApiKey">
              · 🔑 已存密钥
            </template>
          </p>
          <p
            v-if="testResults[provider.id]"
            class="test-result"
            :class="testResults[provider.id]!.ok ? 'test-ok' : 'test-fail'"
          >
            <template v-if="testResults[provider.id]!.ok">
              ✓ 连接成功（{{ testResults[provider.id]!.latencyMs }}ms）{{ testResults[provider.id]!.reply }}
            </template>
            <template v-else>
              ✗ {{ testResults[provider.id]!.error }}
            </template>
          </p>
        </div>
        <div class="provider-actions">
          <button
            class="btn btn-small btn-secondary"
            type="button"
            :disabled="testing[provider.id]"
            @click="store.runTest(provider.id)"
          >
            {{ testing[provider.id] ? "测试中…" : "测试" }}
          </button>
          <button
            class="btn btn-small btn-secondary"
            type="button"
            @click="store.openEdit(provider)"
          >
            编辑
          </button>
          <button
            class="btn btn-small btn-danger"
            type="button"
            @click="store.remove(provider.id)"
          >
            删除
          </button>
        </div>
      </li>
    </ul>
    <p
      v-else-if="!form.open"
      class="muted"
    >
      还没有 Provider，点击「添加」配置第一个（例如本地 Ollama：
      <code>http://localhost:11434/v1</code>）。
    </p>

    <!-- 表单 -->
    <form
      v-if="form.open"
      class="settings-form"
      @submit.prevent="store.save()"
    >
      <div class="field-row">
        <label class="field">
          <span>ID{{ form.isNew ? "（英文标识，保存后不可改）" : "" }}</span>
          <input
            v-model="form.id"
            type="text"
            placeholder="deepseek / local-ollama"
            spellcheck="false"
            :disabled="!form.isNew"
            required
          >
        </label>
        <label class="field">
          <span>名称</span>
          <input
            v-model="form.name"
            type="text"
            placeholder="DeepSeek"
            spellcheck="false"
          >
        </label>
      </div>

      <label class="field">
        <span>接口地址 (baseUrl)</span>
        <input
          v-model="form.baseUrl"
          type="text"
          placeholder="https://api.deepseek.com/v1 或 http://localhost:11434/v1"
          spellcheck="false"
          required
        >
      </label>

      <div class="field-row">
        <label class="field">
          <span>模型</span>
          <input
            v-model="form.model"
            type="text"
            placeholder="deepseek-chat / qwen2.5:7b …"
            spellcheck="false"
          >
        </label>
        <label class="field">
          <span>API Key</span>
          <input
            v-model="form.apiKeyInput"
            type="password"
            autocomplete="off"
            placeholder="留空表示不修改"
          >
        </label>
        <label class="field">
          <span>超时（秒）</span>
          <input
            v-model.number="form.timeoutSecs"
            type="number"
            min="5"
            max="600"
          >
        </label>
      </div>

      <label class="field">
        <span>
          厂商扩展参数（JSON 对象，逐键合并进请求体。例：智谱 GLM 关闭深度思考
          {"thinking":{"type":"disabled"}}；OpenAI o 系列推理力度
          {"reasoning_effort":"low"}。留空不发送）
        </span>
        <textarea
          v-model="form.extraBody"
          rows="2"
          placeholder="{&quot;thinking&quot;:{&quot;type&quot;:&quot;disabled&quot;}}"
          spellcheck="false"
        />
      </label>

      <div class="form-footer">
        <button
          class="btn"
          type="submit"
          :disabled="saving"
        >
          {{ saving ? "保存中…" : "保存" }}
        </button>
        <button
          class="btn btn-secondary"
          type="button"
          @click="store.closeForm()"
        >
          取消
        </button>
      </div>
      <p
        v-if="saveError"
        class="error-banner"
      >
        {{ saveError }}
      </p>
    </form>
  </section>
</template>
