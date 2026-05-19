// auth.ts — daemon keystore 잠금 해제 (단일 비밀번호).
// PRD §1: 1 daemon = 1 사람. multi-user 개념 없음. users 테이블·register·JWT 모두 폐기.

const TOKEN_KEY = "xgram_session_token";

function authBase(): string {
  const meta = document.querySelector("meta[name=\"xgram-daemon\"]") as HTMLMetaElement | null;
  return meta?.content || "/api/auth";
}

export async function unlock(password: string): Promise<void> {
  const r = await fetch(`${authBase()}/unlock`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ password }),
  });
  if (!r.ok) {
    const txt = await r.text();
    throw new Error(r.status === 401 ? "비밀번호가 틀렸습니다" : txt || `HTTP ${r.status}`);
  }
  const { session_token } = await r.json();
  localStorage.setItem(TOKEN_KEY, session_token);
}

export async function isUnlocked(): Promise<boolean> {
  const token = localStorage.getItem(TOKEN_KEY);
  if (!token) return false;
  const r = await fetch(`${authBase()}/check`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  return r.ok;
}

export function lock(): void {
  localStorage.removeItem(TOKEN_KEY);
}
