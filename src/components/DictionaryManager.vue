<script setup lang="ts">
import { onMounted, ref } from "vue";
import { storeToRefs } from "pinia";
import { TSV_FORMAT_HINT, useDictionariesStore } from "../stores/dictionaries";

const store = useDictionariesStore();
const { dictionaries, loadError, importError, importMessage, importing, form, languageOptions } =
  storeToRefs(store);

// 两段确认删除（WKWebView 不支持 window.confirm）
const confirmingId = ref<string | null>(null);

function requestRemove(id: string): void {
  if (confirmingId.value === id) {
    confirmingId.value = null;
    void store.remove(id);
    return;
  }
  confirmingId.value = id;
  window.setTimeout(() => {
    if (confirmingId.value === id) {
      confirmingId.value = null;
    }
  }, 3000);
}

onMounted(() => {
  void store.load();
});
</script>

<template>
  <section class="card">
    <div class="card-header-row">
      <h2>词典</h2>
      <button
        v-if="!form.open"
        class="btn btn-small"
        type="button"
        @click="store.openImport()"
      >
        导入词典
      </button>
    </div>
    <p class="muted">
      本地词典优先：翻译单词时命中词典则直接出结果（快、离线、零成本），未命中才走 AI。
    </p>
    <p
      v-if="loadError"
      class="error-banner"
    >
      {{ loadError }}
    </p>

    <!-- 词典列表 -->
    <ul
      v-if="dictionaries.length > 0 && !form.open"
      class="provider-list"
    >
      <li
        v-for="dictionary in dictionaries"
        :key="dictionary.id"
        class="provider-row"
      >
        <div class="provider-info">
          <div class="provider-title">
            <strong>{{ dictionary.name }}</strong>
            <code class="provider-id">{{ dictionary.id }}</code>
          </div>
          <p class="provider-detail">
            {{ dictionary.sourceLanguage }} → {{ dictionary.glossLanguage }} ·
            {{ dictionary.entryCount }} 个词条
          </p>
        </div>
        <div class="provider-actions">
          <button
            class="btn btn-small"
            :class="confirmingId === dictionary.id ? 'btn-danger-solid' : 'btn-danger'"
            type="button"
            @click="requestRemove(dictionary.id)"
          >
            {{ confirmingId === dictionary.id ? "确认删除" : "删除" }}
          </button>
        </div>
      </li>
    </ul>
    <p
      v-else-if="!form.open"
      class="muted"
    >
      还没有导入词典。可以把 CC-CEDICT / ECDICT 等公开词典转成 TSV 后导入。
    </p>

    <!-- 导入表单 -->
    <form
      v-if="form.open"
      class="settings-form"
      @submit.prevent="store.importTsv()"
    >
      <div class="field-row">
        <label class="field">
          <span>ID（英文标识）</span>
          <input
            v-model="form.id"
            type="text"
            placeholder="my-en-zh"
            spellcheck="false"
            required
          >
        </label>
        <label class="field">
          <span>名称</span>
          <input
            v-model="form.name"
            type="text"
            placeholder="我的英汉词典"
            spellcheck="false"
          >
        </label>
      </div>

      <div class="field-row">
        <label class="field">
          <span>词头语言</span>
          <select v-model="form.sourceLanguage">
            <option
              v-for="lang in languageOptions"
              :key="lang.code"
              :value="lang.code"
            >
              {{ lang.label }}（{{ lang.code }}）
            </option>
          </select>
        </label>
        <label class="field">
          <span>释义语言</span>
          <select v-model="form.glossLanguage">
            <option
              v-for="lang in languageOptions"
              :key="lang.code"
              :value="lang.code"
            >
              {{ lang.label }}（{{ lang.code }}）
            </option>
          </select>
        </label>
      </div>

      <label class="field">
        <span>TSV 文件路径</span>
        <input
          v-model="form.path"
          type="text"
          placeholder="/Users/you/dictionary.tsv"
          spellcheck="false"
          required
        >
      </label>
      <p class="format-hint">
        {{ TSV_FORMAT_HINT }}
      </p>

      <div class="form-footer">
        <button
          class="btn"
          type="submit"
          :disabled="importing"
        >
          {{ importing ? "导入中…" : "开始导入" }}
        </button>
        <button
          class="btn btn-secondary"
          type="button"
          @click="store.closeForm()"
        >
          取消
        </button>
        <span
          v-if="importMessage"
          class="save-ok"
        >{{ importMessage }}</span>
      </div>
      <p
        v-if="importError"
        class="error-banner"
      >
        {{ importError }}
      </p>
    </form>
  </section>
</template>
