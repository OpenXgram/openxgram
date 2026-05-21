import { createSignal} from "solid-js";
import { unlock} from "@/api/auth";
import { useI18n} from "@/i18n";

export function LoginView(props: { onUnlock: () => void}) {
 const { t} = useI18n();
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
 <div class="auth-shell">
 <div class="auth-card">
 <div class="auth-header">
 <div class="auth-brand">OpenXgram</div>
 <h1 class="auth-title">{t("auth.unlock.title") || "잠금 해제"}</h1>
 <p class="auth-sub">{t("auth.unlock.sub") || "이 머신의 keystore 비밀번호를 입력하세요"}</p>
 </div>

 <form class="auth-form" onSubmit={handleSubmit}>
 <div class="auth-field">
 <label class="auth-label" for="auth-password">
 {t("auth.password") || "비밀번호"}
 </label>
 <input
 id="auth-password"
 class="auth-input"
 type="password"
 autocomplete="current-password"
 autofocus
 value={password()}
 onInput={(e) => setPassword(e.currentTarget.value)}
 disabled={busy()}
 required
 />
 </div>

 {error() && (
 <div class="auth-error" role="alert">{error()}</div>
)}

 <button type="submit" class="auth-button-primary" disabled={busy() || !password()}>
 {busy() ? (t("common.loading") || "확인 중...") : (t("auth.unlock.button") || "잠금 해제")}
 </button>
 </form>

 <div class="auth-footer">
 <p class="auth-hint">
 init 시 정한 keystore 비밀번호. 분실 시 <code>xgram reset --hard</code> 후 재 init 필요.
 </p>
 </div>
 </div>
 </div>
);
}
