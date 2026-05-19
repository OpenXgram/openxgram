import { createSignal, Show } from "solid-js";
import { useI18n } from "../i18n";
import { register } from "@/api/auth";

const MIN_PASSWORD_LEN = 12;

// 계정 생성 — 이메일 + 비밀번호 (12자+) + alias (선택).
// 회원가입 후 자동 로그인 (서버가 JWT 발급, setBearer 자동).
export function RegisterView(props: {
  onSuccess: () => void;
  onSwitchToLogin: () => void;
}) {
  const { t } = useI18n();
  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [confirm, setConfirm] = createSignal("");
  const [alias, setAlias] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string>("");

  const submit = async (e: Event) => {
    e.preventDefault();
    setError("");
    if (!email().trim() || !password()) {
      setError(t("auth.error.fields_required"));
      return;
    }
    if (password().length < MIN_PASSWORD_LEN) {
      setError(t("auth.error.password_too_short").replace("{n}", String(MIN_PASSWORD_LEN)));
      return;
    }
    if (password() !== confirm()) {
      setError(t("auth.error.password_mismatch"));
      return;
    }
    setBusy(true);
    try {
      await register(email().trim(), password(), alias().trim() || undefined);
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
        <h2>{t("auth.register.title")}</h2>
        <p class="muted">{t("auth.register.subtitle")}</p>
        <form onSubmit={submit}>
          <div class="form-row">
            <label for="reg-email">{t("auth.email")}</label>
            <input
              id="reg-email"
              type="email"
              value={email()}
              onInput={(e) => setEmail(e.currentTarget.value)}
              autocomplete="email"
              required
              style={{ width: "100%" }}
            />
          </div>
          <div class="form-row">
            <label for="reg-password">
              {t("auth.password")} ({t("auth.password.min").replace("{n}", String(MIN_PASSWORD_LEN))})
            </label>
            <input
              id="reg-password"
              type="password"
              value={password()}
              onInput={(e) => setPassword(e.currentTarget.value)}
              autocomplete="new-password"
              minLength={MIN_PASSWORD_LEN}
              required
              style={{ width: "100%" }}
            />
          </div>
          <div class="form-row">
            <label for="reg-confirm">{t("auth.password.confirm")}</label>
            <input
              id="reg-confirm"
              type="password"
              value={confirm()}
              onInput={(e) => setConfirm(e.currentTarget.value)}
              autocomplete="new-password"
              required
              style={{ width: "100%" }}
            />
          </div>
          <div class="form-row">
            <label for="reg-alias">{t("auth.alias")}</label>
            <input
              id="reg-alias"
              type="text"
              value={alias()}
              onInput={(e) => setAlias(e.currentTarget.value)}
              placeholder={t("auth.alias.placeholder")}
              style={{ width: "100%" }}
            />
          </div>
          <div class="form-row" style={{ gap: "8px" }}>
            <button class="primary" type="submit" disabled={busy()}>
              {busy() ? t("common.loading") : t("auth.register.submit")}
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
          {t("auth.register.have_account")}{" "}
          <a
            href="#"
            onClick={(e) => {
              e.preventDefault();
              props.onSwitchToLogin();
            }}
          >
            {t("auth.login.link")}
          </a>
        </p>
      </div>
    </div>
  );
}
