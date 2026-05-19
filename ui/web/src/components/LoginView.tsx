import { createSignal, Show } from "solid-js";
import { useI18n } from "../i18n";
import { login } from "@/api/auth";

// 첫 화면 — 이메일 + 비밀번호 로그인.
// 성공 시 onSuccess() 호출 (App.tsx 가 메인 GUI 로 전환).
// "회원가입" 링크 → onSwitchToRegister().
export function LoginView(props: {
  onSuccess: () => void;
  onSwitchToRegister: () => void;
}) {
  const { t } = useI18n();
  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string>("");

  const submit = async (e: Event) => {
    e.preventDefault();
    setError("");
    if (!email().trim() || !password()) {
      setError(t("auth.error.fields_required"));
      return;
    }
    setBusy(true);
    try {
      await login(email().trim(), password());
      props.onSuccess();
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="auth-shell">
      <div class="card auth-card">
        <h2>{t("auth.login.title")}</h2>
        <p class="muted">{t("auth.login.subtitle")}</p>
        <form onSubmit={submit}>
          <div class="form-row">
            <label for="login-email">{t("auth.email")}</label>
            <input
              id="login-email"
              type="email"
              value={email()}
              onInput={(e) => setEmail(e.currentTarget.value)}
              autocomplete="email"
              required
              style={{ width: "100%" }}
            />
          </div>
          <div class="form-row">
            <label for="login-password">{t("auth.password")}</label>
            <input
              id="login-password"
              type="password"
              value={password()}
              onInput={(e) => setPassword(e.currentTarget.value)}
              autocomplete="current-password"
              required
              style={{ width: "100%" }}
            />
          </div>
          <div class="form-row" style={{ gap: "8px" }}>
            <button class="primary" type="submit" disabled={busy()}>
              {busy() ? t("common.loading") : t("auth.login.submit")}
            </button>
          </div>
        </form>
        <Show when={error()}>
          <p class="hint" style={{ color: "var(--c-danger,#c00)" }}>
            {error()}
          </p>
        </Show>
        <hr style={{ "margin-top": "16px", "margin-bottom": "16px" }} />
        <p class="hint">
          {t("auth.login.no_account")}{" "}
          <a
            href="#"
            onClick={(e) => {
              e.preventDefault();
              props.onSwitchToRegister();
            }}
          >
            {t("auth.register.link")}
          </a>
        </p>
        <p class="hint muted" style={{ "margin-top": "4px" }}>
          {t("auth.forgot.disabled_hint")}
        </p>
      </div>
    </div>
  );
}
