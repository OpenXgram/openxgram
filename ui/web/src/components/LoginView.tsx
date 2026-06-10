import { createSignal } from "solid-js";
import { unlock } from "@/api/auth";
import { useI18n } from "@/i18n";
import "./kakao.css";

export function LoginView(props: { onUnlock: () => void }) {
  const { t } = useI18n();
  const [password, setPassword] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  async function handleSubmit(e: Event) {
    e.preventDefault();
    setError(null);
    if (!password()) return;
    setBusy(true);
    try {
      await unlock(password());
      props.onUnlock();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="kk-login">
      <div class="kk-login-card">
        <div class="kk-login-brandmark" aria-hidden="true">X</div>
        <div class="kk-login-brand">OpenXgram</div>
        <h1 class="kk-login-title">{t("auth.unlock.title") || "잠금 해제"}</h1>
        <p class="kk-login-sub">
          {t("auth.unlock.sub") || "이 머신의 keystore 비밀번호를 입력하세요"}
        </p>

        <form class="kk-login-form" onSubmit={handleSubmit}>
          <label class="kk-login-label" for="auth-password">
            {t("auth.password") || "비밀번호"}
          </label>
          <input
            id="auth-password"
            class="kk-login-input"
            type="password"
            autocomplete="current-password"
            autofocus
            placeholder={t("auth.password") || "비밀번호"}
            value={password()}
            onInput={(e) => setPassword(e.currentTarget.value)}
            disabled={busy()}
            required
          />

          {error() && (
            <div class="kk-login-error" role="alert">
              {error()}
            </div>
          )}

          <button
            type="submit"
            class="kk-login-btn"
            disabled={busy() || !password()}
          >
            {busy()
              ? t("common.loading") || "확인 중..."
              : t("auth.unlock.button") || "잠금 해제"}
          </button>
        </form>

        <div class="kk-login-foot">
          init 시 정한 keystore 비밀번호. 분실 시 <code>xgram reset --hard</code> 후 재 init 필요.
        </div>
      </div>
    </div>
  );
}
