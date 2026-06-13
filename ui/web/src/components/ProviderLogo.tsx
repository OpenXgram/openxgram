import { Show } from "solid-js";

// AI 제공자 로고 — 인라인 SVG 브랜드 마크(텍스트 라벨 아님). ~16px, 카드 1줄 칩 자리에 들어감.
//   claude: 주황(#D97757) sunburst · openai: 회색 매듭 · gemini: 파랑→보라 sparkle · ollama: 라마 실루엣.
//   title 로 hover 시 제공자명 확인.
// TalkTab·AgentsTab 공유 — providerKey 로 ai_type 을 정규화한 뒤 이 컴포넌트로 렌더.

export type ProviderId = "claude" | "openai" | "gemini" | "ollama";

const PROVIDER_NAME: Record<string, string> = {
  claude: "Claude", openai: "OpenAI", gemini: "Gemini", ollama: "Ollama",
};

// AI 제공자 키 정규화 — ai_type 기준(model 필드 아님). 비었거나 "default" 면 claude 폴백.
//   매핑: claude(Anthropic) · openai(gpt/codex) · gemini(Google) · ollama. 미지/빈값 → claude.
export function providerKey(a: { ai_type?: string | null }): ProviderId {
  const t = (a.ai_type ?? "").trim().toLowerCase();
  if (!t || t === "default") return "claude"; // ai_type 비었으면 현재 전부 claude.
  if (t === "claude" || t === "anthropic") return "claude";
  if (t === "openai" || t === "gpt" || t === "codex") return "openai";
  if (t === "gemini" || t === "google") return "gemini";
  if (t === "ollama") return "ollama";
  return "claude"; // hermes 등 기타도 로컬/claude 계열로 폴백(텍스트 노출 금지).
}

export function ProviderLogo(p: { provider: ProviderId; size?: number }) {
  const s = p.size ?? 16;
  const name = PROVIDER_NAME[p.provider] ?? p.provider;
  return (
    <span class="kk-card-logo" title={name} aria-label={name} style={`width:${s}px;height:${s}px`}>
      <Show when={p.provider === "claude"}>
        {/* Anthropic sunburst — 주황 별표(8방향 spike). */}
        <svg viewBox="0 0 24 24" width={s} height={s} fill="#D97757" aria-hidden="true">
          <path d="M12 1.2l1.7 6.1 4.4-4.5-2.2 6 6.1-1.7-5.4 3.2 5.4 3.2-6.1-1.7 2.2 6-4.4-4.5L12 22.8l-1.7-6.1-4.4 4.5 2.2-6-6.1 1.7 5.4-3.2L2 11.5l6.1 1.7-2.2-6 4.4 4.5L12 1.2z" />
        </svg>
      </Show>
      <Show when={p.provider === "openai"}>
        {/* OpenAI knot — 회색 6-lobe 매듭(단순화 인식 마크). */}
        <svg viewBox="0 0 24 24" width={s} height={s} fill="none" stroke="#6b7280" stroke-width="1.7" aria-hidden="true">
          <path d="M12 3.2a4.2 4.2 0 0 1 3.64 2.1 4.2 4.2 0 0 1 1.86 7.32 4.2 4.2 0 0 1-3.64 6.18 4.2 4.2 0 0 1-7.28 0A4.2 4.2 0 0 1 2.5 12.62 4.2 4.2 0 0 1 6.36 5.3 4.2 4.2 0 0 1 12 3.2z" />
          <path d="M12 8.2v7.6M8.6 9.9l6.8 4M8.6 14.1l6.8-4" stroke-width="1.3" />
        </svg>
      </Show>
      <Show when={p.provider === "gemini"}>
        {/* Gemini 4-point sparkle — 파랑→보라 그라데이션. */}
        <svg viewBox="0 0 24 24" width={s} height={s} aria-hidden="true">
          <defs>
            <linearGradient id="kk-gemini-grad" x1="0" y1="0" x2="1" y2="1">
              <stop offset="0%" stop-color="#4285F4" />
              <stop offset="100%" stop-color="#9B72CB" />
            </linearGradient>
          </defs>
          <path d="M12 1.5c.3 4.7 1.8 8.7 4.6 11.5-2.8 .6-4.3 2.2-4.6 7-.3-4.8-1.8-6.4-4.6-7C10.2 10.2 11.7 6.2 12 1.5z" fill="url(#kk-gemini-grad)" />
        </svg>
      </Show>
      <Show when={p.provider === "ollama"}>
        {/* Ollama llama 실루엣 — 검정 단순 마크. */}
        <svg viewBox="0 0 24 24" width={s} height={s} fill="#111827" aria-hidden="true">
          <path d="M7 2.4c.9 0 1.4.9 1.5 2.1.1 1 .1 2 .1 2.8h6.8c0-.8 0-1.8.1-2.8.1-1.2.6-2.1 1.5-2.1.9 0 1.4 1 1.4 2.4 0 1.1-.2 2.4-.5 3.5.7.8 1.1 1.9 1.1 3.3v5.5c0 1.6-.7 2.6-2 2.6-1 0-1.6-.6-1.8-1.6h-4.4c-.2 1-.8 1.6-1.8 1.6-1.3 0-2-1-2-2.6V11.6c0-1.4.4-2.5 1.1-3.3-.3-1.1-.5-2.4-.5-3.5 0-1.4.5-2.4 1.4-2.4zm2.6 9.2a1 1 0 1 0 0 2 1 1 0 0 0 0-2zm4.8 0a1 1 0 1 0 0 2 1 1 0 0 0 0-2z" />
        </svg>
      </Show>
    </span>
  );
}
