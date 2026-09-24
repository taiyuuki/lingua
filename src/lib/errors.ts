/**
 * 错误展示层：把 Rust `AppError`（`{kind, message}`）与普通异常
 * 转成用户可读文案。组件层不感知错误结构。
 */

export interface AppErrorPayload {
  kind: string;
  message: string;
}

const KIND_LABELS: Record<string, string> = {
  network: "网络错误",
  authentication: "认证失败，请检查 API Key",
  provider: "AI 服务返回错误",
  database: "本地数据库错误",
  ocr: "OCR 识别失败",
  permission: "缺少系统权限",
  platform: "平台相关错误",
  invalid_input: "输入不合法",
  timeout: "请求超时",
  internal: "内部错误",
};

export function describeError(error: unknown): string {
  if (typeof error === "string") {
    // Tauri 命令 reject 的字符串错误（如 setup 阶段失败）
    return error;
  }
  if (isAppErrorPayload(error)) {
    const label = KIND_LABELS[error.kind] ?? "未知错误";
    return `${label}：${error.message}`;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

function isAppErrorPayload(value: unknown): value is AppErrorPayload {
  return (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    typeof (value as Record<string, unknown>).kind === "string" &&
    "message" in value &&
    typeof (value as Record<string, unknown>).message === "string"
  );
}
