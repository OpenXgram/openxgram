// 공유 세션 유틸 — TalkTab·AgentsTab 의 "추가되지 않은 에이전트"(미등록 tmux 감지) 로직을
// 단일 출처로 모은 모듈. 프레임워크 비의존(순수 함수) — SolidJS memo 가 이 함수들을 감싼다.
//
// sessions 라우트(SessionsDto) — Messenger.tsx 와 동일 contract. 이 에이전트의 tmux 세션·워크트리 소스.
export interface DetectedSession {
  kind: "tmux" | "claude_project" | "xgram_session";
  identifier: string;
  display: string;
  status: "active" | "attached" | "detached" | "stale";
  windows: number | null;
  attached: boolean | null;
  created_at: string | null;
  last_active_at: string | null;
  agent_id: string | null;
  // rc.228 — 세션에 nested 된 git worktree (path/branch). 패널 워크트리 섹션 소스.
  worktrees?: { path: string; branch?: string | null }[];
  // rc.281 — 이 tmux 세션 active pane 의 작업 폴더(`#{pane_current_path}`). cwd 매칭 소스.
  cwd?: string | null;
}

export interface SessionsDto {
  machine: { hostname: string; alias: string; tailscale_ip: string | null };
  sessions: DetectedSession[];
}

// 경로 끝의 슬래시 제거(정규화). cwd / project_path 비교 전 표준화.
export function normPath(p: string): string {
  return p.replace(/\/+$/, "");
}

// `~` / `~/...` 를 home 절대경로로 확장. project_path 에 tilde 가 저장되는 경우가 있어
//   (예: "~/projects/starian-set") cwd("/home/llm/projects/starian-set") 매칭 전 반드시 확장.
//   home 은 프론트에서 env 를 못 읽으므로 호출부가 세션 cwd 들에서 추정한 값을 주입한다.
export function expandHome(p: string, home: string): string {
  const t = (p ?? "").trim();
  if (!t) return "";
  const h = normPath((home ?? "").trim());
  if (!h) return t;
  if (t === "~") return h;
  if (t.startsWith("~/")) return h + t.slice(1); // "~/x" -> "<home>/x"
  return t;
}

// 세션 cwd 들에서 home 루트(`/home/<user>` 또는 `/Users/<user>`)를 추정.
//   가장 흔하게 등장하는 home prefix 를 채택(동률이면 첫 등장). 없으면 "".
export function detectHome(cwds: (string | null | undefined)[]): string {
  const counts = new Map<string, number>();
  for (const raw of cwds) {
    const c = normPath((raw ?? "").trim());
    const m = c.match(/^(\/home\/[^/]+|\/Users\/[^/]+)(?:\/|$)/);
    if (m) counts.set(m[1], (counts.get(m[1]) ?? 0) + 1);
  }
  let best = "";
  let bestN = 0;
  for (const [h, n] of counts) if (n > bestN) { best = h; bestN = n; }
  return best;
}

// alias 정규화 — 공백·특수문자 제거 + 소문자. "Starian Set" -> "starianset".
//   세션 식별자에도 동일 적용 후 substring 포함 검사에 사용.
export function normAlias(s: string): string {
  return (s ?? "").toLowerCase().replace(/[^a-z0-9]+/g, "");
}

// 정규화 alias 가 세션 이름(identifier/display/agent_id)에 substring 으로 포함되는가.
//   "starianset" ∈ normAlias("aoe_starianset_944a15df")=="aoestarianset944a15df" → true.
//   너무 짧은 alias(<=2)는 오매칭 방지로 제외.
export function aliasMatchesSession(alias: string, s: DetectedSession): boolean {
  const a = normAlias(alias);
  if (a.length < 3) return false;
  for (const cand of [s.agent_id, s.display, s.identifier]) {
    const c = normAlias(cand ?? "");
    if (c && c.includes(a)) return true;
  }
  return false;
}

// 의미있는 작업 tmux 만: aoe_* 세션이거나, cwd 가 실제 프로젝트 폴더(HOME 하위·루트/시스템 아님).
export function isMeaningfulSession(s: DetectedSession): boolean {
  if (s.kind !== "tmux") return false;
  const ident = (s.identifier ?? "").trim();
  const disp = (s.display ?? "").trim();
  const cwd = (s.cwd ?? "").trim();
  // 데몬 자기 세션·시스템 류 제외(이름 기준).
  const nameNoise = /^(null|default|\d+|server|main|0|bash|sh)$/i;
  if (!ident || nameNoise.test(ident)) return false;
  // aoe_* 는 항상 작업 에이전트 세션으로 간주.
  if (/^aoe[_-]/i.test(ident) || /^aoe[_-]/i.test(disp)) return true;
  // 그 외엔 cwd 가 실제 프로젝트 폴더여야(루트·HOME 직속·/tmp 등 제외).
  if (!cwd) return false;
  const c = normPath(cwd);
  if (c === "/" || c === "" || c === "/home/llm" || c === "/root" || c.startsWith("/tmp")) return false;
  if (!c.startsWith("/home/") && !c.startsWith("/opt/") && !c.startsWith("/srv/")) return false;
  return true;
}

// 홈 루트급(너무 넓은) 경로 — 이런 게 등록 경로면 그 아래 모든 tmux 가 "등록됨"으로
//   흡수돼 미등록 섹션이 영영 비어버린다(prefix-ownership leak). 매칭 set 에서 제외.
//   예: `/`, `/home/<user>`, `/Users/<user>`, `/root`, `/home`.
export function isTooBroadPath(p: string): boolean {
  const c = normPath(p);
  if (c === "" || c === "/" || c === "/root" || c === "/home") return true;
  return /^\/home\/[^/]+$/.test(c) || /^\/Users\/[^/]+$/.test(c);
}

// ➕ "추가되지 않은 에이전트" — detect_tmux(sessions) 의 tmux 세션 중 어느 에이전트의
//   project_path(cwd) 와도 안 맞는 것 = 미등록. noise(데몬 자기 세션·null·시스템 세션) 제외.
//   agentProjectPaths = 등록된 에이전트들의 project_path 목록(빈 값 제외). 순수 함수.
export function computeUnregisteredSessions(
  sessions: DetectedSession[],
  agentProjectPaths: string[],
): DetectedSession[] {
  const regs = new Set<string>();
  for (const p of agentProjectPaths) {
    const t = (p ?? "").trim();
    // 홈 루트급 경로는 제외 — 안 그러면 홈 아래 전부를 흡수해 미등록이 항상 0.
    if (t && !isTooBroadPath(t)) regs.add(normPath(t));
  }
  const out: DetectedSession[] = [];
  const seen = new Set<string>();
  for (const s of sessions) {
    if (!isMeaningfulSession(s)) continue;
    const cwd = s.cwd ? normPath(s.cwd.trim()) : "";
    // 이미 등록된 에이전트 폴더면 제외(그 폴더·하위면 등록된 것으로 본다).
    if (cwd && [...regs].some((r) => cwd === r || cwd.startsWith(r + "/"))) continue;
    const key = s.identifier;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(s);
  }
  return out;
}
