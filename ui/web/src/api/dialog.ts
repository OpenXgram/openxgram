// Tauri plugin-dialog shim — 브라우저 confirm/alert.
//
// Tauri 의 `ask()` 두 번째 인자는 `{ kind?: "info" | "warning" | "error" }`
// 등 옵션 객체이지만, 브라우저 confirm() 은 옵션 미지원. 호환을 위해
// 두 번째 인자는 무시한다.

type AskOptions = {
  kind?: "info" | "warning" | "error";
  title?: string;
  okLabel?: string;
  cancelLabel?: string;
};

export async function ask(
  messageText: string,
  _options?: AskOptions,
): Promise<boolean> {
  return window.confirm(messageText);
}

export async function message(
  messageText: string,
  _options?: AskOptions,
): Promise<void> {
  window.alert(messageText);
}
