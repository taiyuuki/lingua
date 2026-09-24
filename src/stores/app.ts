import { ref } from "vue";
import { defineStore } from "pinia";
import {
  getAppInfo,
  getDatabaseStatus,
  openPopup,
  type AppInfo,
  type DatabaseStatus,
} from "../lib/api";
import { describeError } from "../lib/errors";

/**
 * 应用级状态（运行信息、数据库状态、全局错误提示）。
 *
 * 状态管理约定：按域拆 store（后续将有 settings / translation /
 * vocabulary / history / provider 等），store 只做 state + 调用
 * `lib/api`，禁止出现 AI 请求、SQL、OCR 等实现细节。
 */
export const useAppStore = defineStore("app", () => {
  const appInfo = ref<AppInfo | null>(null);
  const database = ref<DatabaseStatus | null>(null);
  const errorMessage = ref<string | null>(null);
  const loading = ref(false);

  async function refresh(): Promise<void> {
    loading.value = true;
    errorMessage.value = null;
    try {
      appInfo.value = await getAppInfo();
      database.value = await getDatabaseStatus();
    } catch (error) {
      errorMessage.value = describeError(error);
    } finally {
      loading.value = false;
    }
  }

  async function openPopupWindow(): Promise<void> {
    errorMessage.value = null;
    try {
      await openPopup();
    } catch (error) {
      errorMessage.value = describeError(error);
    }
  }

  return { appInfo, database, errorMessage, loading, refresh, openPopupWindow };
});
