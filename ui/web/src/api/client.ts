// Web GUI — Tauri invoke() shim.
//
// 기존 컴포넌트는 `import { invoke } from "@tauri-apps/api/core"` 사용.
// Web 빌드에서는 `import { invoke } from "@/api/client"` 로 1줄 교체.
//
// daemon HTTP API 는 REST 스타일 (GET/POST/PUT/DELETE + 경로). Tauri 의
// `invoke(name, args)` (모두 POST + command name) 패턴과 다르므로 이 모듈에서
// command name → {method, path-template} 로 변환한다. 경로의 {id} 같은
// placeholder 는 args 에서 동일 키로 채운다.
//
// 라우팅 표는 daemon_gui.rs (Router::new()) 와 1:1 대응.
// daemon 에 없는 명령은 Error 던짐 (UI 가 에러 메시지 표시).

// daemon 이 직접 /gui/ 정적 자산 서빙하므로 same-origin /v1/gui/* 그대로 호출.
// nginx reverse proxy 있으면 그쪽도 /v1/gui/* pass-through.
// 다른 호스트의 daemon 사용 시 Settings 탭에서 절대 URL 입력.
const DEFAULT_BASE = "/v1/gui";
const LEGACY_BASE = "/api/gui"; // pre-rc.26 default — 자동 마이그레이션

const URL_KEY = "xgram_daemon_url";
const TOKEN_KEY = "xgram_mcp_token";

export function getDaemonUrl(): string {
  try {
    const stored = localStorage.getItem(URL_KEY);
    // rc.26 마이그레이션: 옛 default 가 저장돼 있으면 무시 → 새 default.
    if (!stored || stored === LEGACY_BASE) return DEFAULT_BASE;
    return stored;
  } catch {
    return DEFAULT_BASE;
  }
}

export function setDaemonUrl(url: string): void {
  try {
    if (url.trim()) {
      localStorage.setItem(URL_KEY, url.trim());
    } else {
      localStorage.removeItem(URL_KEY);
    }
  } catch {
    // ignored — private mode
  }
}

export function getBearer(): string | null {
  // 우선순위: session_token (웹 GUI unlock) > mcp_token (CLI 발급).
  // 두 키가 분리된 이유: unlock 토큰은 daemon 프로세스 수명, mcp-token 은 영구.
  // require_auth 핸들러는 둘 다 받음.
  try {
    return (
      localStorage.getItem("xgram_session_token") ||
      localStorage.getItem(TOKEN_KEY)
    );
  } catch {
    return null;
  }
}

export function setBearer(token: string): void {
  try {
    if (token.trim()) {
      localStorage.setItem(TOKEN_KEY, token.trim());
    } else {
      localStorage.removeItem(TOKEN_KEY);
    }
  } catch {
    // ignored
  }
}

type HttpMethod = "GET" | "POST" | "PUT" | "DELETE";

interface Route {
  method: HttpMethod;
  /** path 템플릿; `{id}` 등은 args 에서 같은 키로 치환. */
  path: string;
  /** path placeholder 채운 후 남는 args 키를 body 로 보낼지 (POST/PUT 기본 true). */
  body?: boolean;
  /** 응답 본문이 비어있으면 이 값을 반환 (기본 undefined). */
  emptyAs?: unknown;
}

// daemon_gui.rs Router::new() 에 정의된 엔드포인트와 1:1 매핑.
const ROUTES: Record<string, Route> = {
  // 기본 상태
  status: { method: "GET", path: "/status" },
  is_initialized: { method: "GET", path: "/initialized" },
  health: { method: "GET", path: "/health" },

  // Peers
  peers_list: { method: "GET", path: "/peers", emptyAs: [] },
  peer_add: { method: "POST", path: "/peers", body: true },

  // Channel
  channel_status: { method: "GET", path: "/channel/status" },

  // Vault
  vault_pending_list: { method: "GET", path: "/vault/pending", emptyAs: [] },
  vault_pending_approve: {
    method: "POST",
    path: "/vault/pending/{id}/approve",
  },
  vault_pending_deny: {
    method: "POST",
    path: "/vault/pending/{id}/deny",
    body: true,
  },

  // Payment limit
  payment_get_daily_limit: { method: "GET", path: "/payment/daily-limit" },
  payment_set_daily_limit: {
    method: "PUT",
    path: "/payment/daily-limit",
    body: true,
  },

  // Notify
  notify_status: { method: "GET", path: "/notify/status" },
  notify_discord_validate: {
    method: "POST",
    path: "/notify/discord/validate",
    body: true,
  },
  notify_discord_guilds: {
    method: "POST",
    path: "/notify/discord/guilds",
    body: true,
  },
  notify_discord_save: {
    method: "POST",
    path: "/notify/discord/save",
    body: true,
  },
  notify_telegram_validate: {
    method: "POST",
    path: "/notify/telegram/validate",
    body: true,
  },
  notify_telegram_detect_chat: {
    method: "POST",
    path: "/notify/telegram/detect_chat",
    body: true,
  },
  notify_telegram_save: {
    method: "POST",
    path: "/notify/telegram/save",
    body: true,
  },

  // Schedule
  schedule_list: { method: "GET", path: "/schedule", emptyAs: [] },
  schedule_create: { method: "POST", path: "/schedule", body: true },
  schedule_stats: { method: "GET", path: "/schedule/stats" },
  schedule_cancel: { method: "POST", path: "/schedule/{id}/cancel" },

  // Chain
  chain_list: { method: "GET", path: "/chain", emptyAs: [] },
  chain_delete: { method: "DELETE", path: "/chain/{name}" },
  // chain_show 는 컴포넌트에서 직접 호출 안 함 (chain_list 가 dto 다 줌).

  // 메신저 v1.3 Step 0 — 메시지 송수신
  messages_recent: { method: "GET", path: "/messages", emptyAs: [] },
  peer_send: { method: "POST", path: "/peers/{alias}/send", body: true },
};

/** path 템플릿 치환 + 남은 args 반환. */
function renderPath(
  template: string,
  args: Record<string, unknown> | undefined,
): { path: string; remaining: Record<string, unknown> } {
  if (!args) return { path: template, remaining: {} };
  const remaining: Record<string, unknown> = { ...args };
  const path = template.replace(/\{(\w+)\}/g, (_m, key: string) => {
    const v = remaining[key];
    if (v === undefined || v === null) {
      throw new Error(`invoke: path placeholder {${key}} 누락`);
    }
    delete remaining[key];
    return encodeURIComponent(String(v));
  });
  return { path, remaining };
}

/**
 * invoke shim — Tauri 코어의 `invoke()` 와 동일한 signature.
 *
 * @param command  daemon GUI 명령 (예: "peers_list", "vault_pending_approve").
 * @param args     path placeholder + body. POST/PUT 에선 path placeholder 외 모든
 *                 키가 JSON body 로 전송됨.
 * @throws Error("미인증 ...") on 401.
 * @throws Error("HTTP NNN: ...") on non-2xx.
 * @throws Error("invoke: ... 미지원 ...") on unknown command.
 */
export async function invoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const route = ROUTES[command];
  if (!route) {
    throw new Error(
      `invoke: 명령 '${command}' 은(는) Web GUI 에서 미지원. ` +
        `(daemon REST API 미존재 — Tauri 빌드만 가능)`,
    );
  }

  const { path, remaining } = renderPath(route.path, args);
  const base = getDaemonUrl().replace(/\/+$/, "");
  let url = `${base}${path}`;
  const headers: Record<string, string> = {};
  const token = getBearer();
  if (token) {
    headers["Authorization"] = `Bearer ${token}`;
  }

  let body: string | undefined;
  if (route.body && (route.method === "POST" || route.method === "PUT")) {
    headers["Content-Type"] = "application/json";
    body = JSON.stringify(remaining);
  } else if (
    Object.keys(remaining).length > 0 &&
    (route.method === "POST" || route.method === "PUT")
  ) {
    // body:true 가 false 여도 POST/PUT 에 잔여 args 있으면 body 로 전송 (안전 기본).
    headers["Content-Type"] = "application/json";
    body = JSON.stringify(remaining);
  } else if (
    Object.keys(remaining).length > 0 &&
    (route.method === "GET" || route.method === "DELETE")
  ) {
    // GET/DELETE 의 잔여 args 는 query string 으로 전송.
    const qs = new URLSearchParams(
      Object.entries(remaining).map(([k, v]) => [k, String(v)]),
    ).toString();
    url += (url.includes("?") ? "&" : "?") + qs;
  }

  let res: Response;
  try {
    res = await fetch(url, { method: route.method, headers, body });
  } catch (e) {
    throw new Error(
      `daemon 연결 실패 (${url}) — daemon 가동 + URL 확인: ${(e as Error).message}`,
    );
  }

  if (res.status === 401) {
    // 세션 만료/위조 — 로컬 Bearer 삭제 후 throw. App.tsx 가 LoginView 로 복귀.
    try {
      localStorage.removeItem(TOKEN_KEY);
    } catch {
      // ignored
    }
    throw new Error("미인증 — 다시 로그인하세요");
  }
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`HTTP ${res.status}: ${text || res.statusText}`);
  }

  const text = await res.text();
  if (!text) {
    return (route.emptyAs ?? (undefined as unknown)) as T;
  }
  try {
    return JSON.parse(text) as T;
  } catch {
    return text as unknown as T;
  }
}
