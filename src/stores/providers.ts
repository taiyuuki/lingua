import { computed, reactive, ref } from "vue";
import { defineStore } from "pinia";
import {
  deleteProvider,
  listProviders,
  saveProvider,
  testProvider,
  type ProviderDto,
  type TestProviderResult,
} from "../lib/api";
import { describeError } from "../lib/errors";

/** 表单的可编辑副本；`id` 仅在新建时可改。 */
export interface ProviderForm {
  open: boolean;
  isNew: boolean;
  id: string;
  name: string;
  baseUrl: string;
  model: string;
  apiKeyInput: string;
  timeoutSecs: number;
  disableThinking: boolean;
}

function emptyForm(): ProviderForm {
  return {
    open: false,
    isNew: true,
    id: "",
    name: "",
    baseUrl: "",
    model: "",
    apiKeyInput: "",
    timeoutSecs: 60,
    disableThinking: false,
  };
}

/**
 * Provider 管理域状态：列表、编辑表单、连接测试结果。
 * store 只做 state + 调 `lib/api`。
 */
export const useProvidersStore = defineStore("providers", () => {
  const providers = ref<ProviderDto[]>([]);
  const loaded = ref(false);
  const loadError = ref<string | null>(null);
  const saveError = ref<string | null>(null);
  const saving = ref(false);

  const form = reactive<ProviderForm>(emptyForm());

  const testing = reactive<Record<string, boolean>>({});
  const testResults = reactive<Record<string, TestProviderResult>>({});

  const hasProviders = computed(() => providers.value.length > 0);

  async function load(): Promise<void> {
    loadError.value = null;
    try {
      providers.value = await listProviders();
      loaded.value = true;
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  function openCreate(): void {
    Object.assign(form, emptyForm(), { open: true, isNew: true });
    saveError.value = null;
  }

  function openEdit(provider: ProviderDto): void {
    Object.assign(form, {
      open: true,
      isNew: false,
      id: provider.id,
      name: provider.name,
      baseUrl: provider.baseUrl,
      model: provider.model ?? "",
      apiKeyInput: "",
      timeoutSecs: provider.timeoutSecs,
      disableThinking: provider.disableThinking,
    });
    saveError.value = null;
  }

  function closeForm(): void {
    form.open = false;
  }

  async function save(): Promise<void> {
    saving.value = true;
    saveError.value = null;
    try {
      await saveProvider({
        id: form.id.trim(),
        name: form.name.trim() || form.id.trim(),
        kind: "openai_compatible",
        baseUrl: form.baseUrl.trim(),
        model: form.model.trim() || null,
        organization: null,
        timeoutSecs: Math.min(Math.max(Math.round(form.timeoutSecs) || 60, 5), 600),
        disableThinking: form.disableThinking,
        apiKey: form.apiKeyInput === "" ? null : form.apiKeyInput,
        hasApiKey: false,
      });
      form.open = false;
      await load();
    } catch (error) {
      saveError.value = describeError(error);
    } finally {
      saving.value = false;
    }
  }

  async function remove(id: string): Promise<void> {
    if (!window.confirm(`确定删除 Provider「${id}」？其 API Key 也会一并清除。`)) {
      return;
    }
    try {
      await deleteProvider(id);
      delete testResults[id];
      await load();
    } catch (error) {
      loadError.value = describeError(error);
    }
  }

  async function runTest(id: string): Promise<void> {
    testing[id] = true;
    try {
      testResults[id] = await testProvider(id);
    } catch (error) {
      testResults[id] = { ok: false, latencyMs: null, reply: null, error: describeError(error) };
    } finally {
      testing[id] = false;
    }
  }

  return {
    providers,
    loaded,
    loadError,
    saveError,
    saving,
    form,
    testing,
    testResults,
    hasProviders,
    load,
    openCreate,
    openEdit,
    closeForm,
    save,
    remove,
    runTest,
  };
});
