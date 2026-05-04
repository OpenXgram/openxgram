import { createSignal, onCleanup, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { writeText, clear } from "@tauri-apps/plugin-clipboard-manager";
import { useI18n } from "../i18n";

const REVEAL_TTL_MS = 30_000;

export function VaultRevealView() {
  const { t } = useI18n();
  const [key, setKey] = createSignal("");
  const [revealed, setRevealed] = createSignal<string | null>(null);
  const [revealUntil, setRevealUntil] = createSignal<number | null>(null);
  let revealTimer: ReturnType<typeof setTimeout> | undefined;
  let clipboardTimer: ReturnType<typeof setTimeout> | undefined;

  const reveal = async () => {
    if (!key()) return;
    // ephemeral token — Stronghold 내 임시 저장; webview 는 토큰만 받고 직접 plaintext 미보관 권장.
    // 데모 단계에선 plaintext 직접 표시.
    const value = await invoke<string>("vault_get", { key: key() });
    setRevealed(value);
    setRevealUntil(Date.now() + REVEAL_TTL_MS);
    if (revealTimer !== undefined) clearTimeout(revealTimer);
    revealTimer = setTimeout(() => {
      setRevealed(null);
      setRevealUntil(null);
    }, REVEAL_TTL_MS);
  };

  const copy = async () => {
    if (!revealed()) return;
    await writeText(revealed()!);
    if (clipboardTimer !== undefined) clearTimeout(clipboardTimer);
    clipboardTimer = setTimeout(() => {
      void clear();
    }, REVEAL_TTL_MS);
  };

  onCleanup(() => {
    if (revealTimer !== undefined) clearTimeout(revealTimer);
    if (clipboardTimer !== undefined) clearTimeout(clipboardTimer);
    void clear();
  });

  const remainingSecs = () => {
    const until = revealUntil();
    if (!until) return 0;
    return Math.max(0, Math.ceil((until - Date.now()) / 1000));
  };

  return (
    <div>
      <input
        placeholder="vault key (예: github/token)"
        value={key()}
        onInput={(e) => setKey(e.currentTarget.value)}
        style="width: 100%; padding: 6px;"
      />
      <div style="margin: 8px 0;">
        <button onClick={() => void reveal()} disabled={!key()}>
          {t("vault.reveal_30s")}
        </button>{" "}
        <Show when={revealed()}>
          <button onClick={() => void copy()}>{t("vault.copy")}</button>
        </Show>
      </div>
      <Show when={revealed()}>
        <pre style="background: #fffbe6; border: 1px dashed #aa8; padding: 8px;">
          {revealed()} ({remainingSecs()}s)
        </pre>
        <small>{t("vault.zeroize_note")}</small>
      </Show>
    </div>
  );
}
