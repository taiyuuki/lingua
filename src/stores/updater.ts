import { computed, ref } from "vue";
import { defineStore } from "pinia";
import {
  checkForUpdates,
  downloadAndInstallUpdate,
  type UpdateManifest,
} from "../lib/api";
import { describeError } from "../lib/errors";

/**
 * 应用更新域状态：检查新版本、下载安装（完成后自动重启到新版本）。
 *
 * `check(true)` 为静默模式（启动时的后台检查），失败不打扰用户；
 * 手动检查失败时记录 `error` 由设置页展示。
 */
export const useUpdaterStore = defineStore("updater", () => {
  const checking = ref(false);
  const downloading = ref(false);
  /** 下载进度（百分比 0-100）。 */
  const progress = ref(0);
  const available = ref<UpdateManifest | null>(null);
  /** 上一次手动检查未发现新版本的时间提示。 */
  const checkedAt = ref<string | null>(null);
  const error = ref<string | null>(null);

  /** 侧边栏「设置」菜单红点依据。 */
  const hasUpdate = computed(() => available.value !== null);

  async function check(silent = false): Promise<void> {
    if (checking.value || downloading.value) {
      return;
    }
    checking.value = true;
    error.value = null;
    try {
      available.value = await checkForUpdates();
      if (!available.value && !silent) {
        checkedAt.value = new Date().toLocaleTimeString();
      }
    } catch (cause) {
      if (!silent) {
        error.value = describeError(cause);
      }
    } finally {
      checking.value = false;
    }
  }

  async function downloadAndInstall(): Promise<void> {
    if (downloading.value || !available.value) {
      return;
    }
    downloading.value = true;
    error.value = null;
    progress.value = 0;
    try {
      await downloadAndInstallUpdate((ratio) => {
        progress.value = Math.round(ratio * 100);
      });
      // relaunch 成功不会走到这里；失败则回滚状态供用户重试。
      downloading.value = false;
    } catch (cause) {
      error.value = describeError(cause);
      downloading.value = false;
    }
  }

  return {
    checking,
    downloading,
    progress,
    available,
    checkedAt,
    error,
    hasUpdate,
    check,
    downloadAndInstall,
  };
});
