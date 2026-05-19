// Web GUI — 사용자 인증 (이메일 + 비밀번호 + JWT).
//
// daemon endpoints:
//   POST /v1/auth/register   { email, password, alias? } → { user_id, email, alias, role, jwt_token }
//   POST /v1/auth/login      { email, password }         → { user_id, ... , jwt_token }
//   GET  /v1/auth/me         (Bearer)                    → { user_id, email, alias, role, machine_alias }
//   POST /v1/auth/logout     (Bearer)                    → 204
//
// 인증 토큰은 localStorage 의 xgram_mcp_token 키에 저장 — 기존 mcp-token 슬롯과 공유한다.
// (require_auth 가 JWT/mcp-token 둘 다 수용하므로, GUI 호출은 어느 쪽이든 통과.)

import { getBearer, getDaemonUrl, setBearer } from "./client";

export interface AuthIssued {
  user_id: string;
  email: string;
  alias: string | null;
  role: string;
  jwt_token: string;
}

export interface AuthMe {
  user_id: string;
  email: string;
  alias: string | null;
  role: string;
  machine_alias: string | null;
}

// /v1/gui base 와 /v1/auth base — getDaemonUrl() 는 보통 "/api/gui" 또는
// "http://host:port/v1/gui" 형태. /v1/auth 로 치환하기 위해 마지막 세그먼트만 갈아끼움.
function authBase(): string {
  const base = getDaemonUrl().replace(/\/+$/, "");
  // "/api/gui" → "/api/auth"  (nginx reverse proxy 가 /api/auth → /v1/auth 로 매핑한다고 가정)
  // "http://host:port/v1/gui" → "http://host:port/v1/auth"
  return base.replace(/\/(v1\/gui|api\/gui)$/, (match) =>
    match.replace("gui", "auth"),
  );
}

async function postJson<T>(
  path: string,
  body: unknown,
  withBearer: boolean,
): Promise<T> {
  const url = `${authBase()}${path}`;
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (withBearer) {
    const t = getBearer();
    if (!t) throw new Error("미인증 — Bearer 토큰 없음");
    headers["Authorization"] = `Bearer ${t}`;
  }
  let res: Response;
  try {
    res = await fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
    });
  } catch (e) {
    throw new Error(
      `daemon 연결 실패 (${url}) — daemon 가동 + URL 확인: ${(e as Error).message}`,
    );
  }
  if (res.status === 401) {
    throw new Error("이메일/비밀번호 불일치 또는 세션 만료");
  }
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`HTTP ${res.status}: ${text || res.statusText}`);
  }
  const text = await res.text();
  if (!text) return undefined as unknown as T;
  return JSON.parse(text) as T;
}

async function getJson<T>(path: string): Promise<T> {
  const url = `${authBase()}${path}`;
  const token = getBearer();
  if (!token) throw new Error("미인증 — Bearer 토큰 없음");
  const res = await fetch(url, {
    method: "GET",
    headers: { Authorization: `Bearer ${token}` },
  });
  if (res.status === 401) {
    throw new Error("세션 만료 — 다시 로그인");
  }
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`HTTP ${res.status}: ${text || res.statusText}`);
  }
  return JSON.parse(await res.text()) as T;
}

export async function register(
  email: string,
  password: string,
  alias?: string,
): Promise<AuthIssued> {
  const out = await postJson<AuthIssued>(
    "/register",
    { email, password, alias: alias?.trim() || null },
    false,
  );
  setBearer(out.jwt_token);
  return out;
}

export async function login(email: string, password: string): Promise<AuthIssued> {
  const out = await postJson<AuthIssued>("/login", { email, password }, false);
  setBearer(out.jwt_token);
  return out;
}

export async function me(): Promise<AuthMe> {
  return getJson<AuthMe>("/me");
}

export async function logout(): Promise<void> {
  try {
    await postJson<void>("/logout", {}, true);
  } catch {
    // 서버가 거부해도 로컬 토큰은 삭제 — 클라이언트는 어쨌든 로그아웃.
  }
  setBearer("");
}

/** JWT 가 유효한지 빠르게 확인 — 401 시 localStorage clear 후 false. */
export async function isAuthenticated(): Promise<boolean> {
  const t = getBearer();
  if (!t) return false;
  try {
    await me();
    return true;
  } catch {
    setBearer("");
    return false;
  }
}
