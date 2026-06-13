import { createSignal, Show } from "solid-js";
import { invoke } from "@/api/client";

// 🔑 비밀번호 변경 (keystore/vault rekey) — daemon POST /v1/gui/change-password.
// 어느 설정 컨테이너(ConfigTab 일반 등)에도 들어가도록 자체 인라인 스타일.
// 단일 출처: ConfigTab·(레거시)SettingsTab 둘 다 이 컴포넌트를 import 한다(중복 방지).
export function SecuritySection() {
  const [oldPw, setOldPw] = createSignal("");
  const [newPw, setNewPw] = createSignal("");
  const [confirmPw, setConfirmPw] = createSignal("");
  const [msg, setMsg] = createSignal<string>("");
  const [busy, setBusy] = createSignal(false);

  const submit = async () => {
    setMsg("");
    if (newPw().length < 8) {
      setMsg("⚠️ 새 비밀번호는 8자 이상이어야 합니다.");
      return;
    }
    if (newPw() !== confirmPw()) {
      setMsg("⚠️ 새 비밀번호와 확인이 일치하지 않습니다.");
      return;
    }
    if (newPw() === oldPw()) {
      setMsg("⚠️ 새 비밀번호가 현재 비밀번호와 동일합니다.");
      return;
    }
    setBusy(true);
    try {
      await invoke("change_password", { old_password: oldPw(), new_password: newPw() });
      setMsg("✅ 비밀번호가 변경되었습니다 — 새 비밀번호로 다시 로그인하세요.");
      setOldPw("");
      setNewPw("");
      setConfirmPw("");
    } catch (e) {
      setMsg(`❌ 변경 실패: ${(e as Error).message}`);
    } finally {
      setBusy(false);
    }
  };

  const inp =
    "width:100%;padding:8px 10px;margin-top:4px;border:1px solid #2a2f3a;border-radius:8px;background:#11151c;color:#e6e6e6;box-sizing:border-box;";
  const lbl = "font-size:12px;color:#cfd6e0;display:block;";

  return (
    <div>
      <div class="wsec" style="margin-top:18px;">🔑 비밀번호 변경</div>
      <p style="font-size:12px;color:#9aa1ad;line-height:1.5;margin:6px 0 10px;">
        keystore 식별 서명키 + 모든 vault 자격증명을 새 비밀번호로 재암호화합니다. 변경 전 자동
        백업되며, 변경 후 새 비밀번호로 다시 로그인하세요.
      </p>
      <div style="display:flex;flex-direction:column;gap:10px;max-width:360px;">
        <label style={lbl}>
          현재 비밀번호
          <input
            type="password"
            value={oldPw()}
            onInput={(e) => setOldPw(e.currentTarget.value)}
            autocomplete="current-password"
            style={inp}
          />
        </label>
        <label style={lbl}>
          새 비밀번호 (8자 이상)
          <input
            type="password"
            value={newPw()}
            onInput={(e) => setNewPw(e.currentTarget.value)}
            autocomplete="new-password"
            style={inp}
          />
        </label>
        <label style={lbl}>
          새 비밀번호 확인
          <input
            type="password"
            value={confirmPw()}
            onInput={(e) => setConfirmPw(e.currentTarget.value)}
            autocomplete="new-password"
            style={inp}
          />
        </label>
        <button class="qbtn" disabled={busy()} onClick={submit} style="align-self:flex-start;">
          {busy() ? "변경 중…" : "비밀번호 변경"}
        </button>
        <Show when={msg()}>
          <div style="font-size:12px;line-height:1.5;">{msg()}</div>
        </Show>
      </div>
    </div>
  );
}
