import { createResource, createSignal, For, Show } from "solid-js";
import { invoke } from "@/api/client";
import { NotifySetup } from "./NotifySetup";
import { Breadcrumb } from "./Breadcrumb";

// UI-CHANNEL-SPEC v1.0 §3 — 📱 채널 카드 (PRD §0 #4: 인간 친화 채널).
// 5 탭: 인박스 / 사람 / 채널 등록 / 라우팅 / 모더레이션.

type Tab = "inbox" | "person" | "register" | "routing" | "moderation";

interface MessageDto {
  id: string;
  sender: string;
  body: string;
  timestamp: string;
}

async function fetchInboxMessages(): Promise<MessageDto[]> {
  try {
    const all = await invoke<MessageDto[]>("messages_recent", { limit: 200 });
    // 인간 채널 sender 만 필터 — discord:* / telegram:* prefix
    return all.filter((m) => /^(discord|telegram|slack):/i.test(m.sender));
  } catch {
    return [];
  }
}

export function ChannelCard(props: { onBack: () => void }) {
  const [tab, setTab] = createSignal<Tab>("inbox");
  const [inbox] = createResource(fetchInboxMessages);

  return (
    <div class="card-page">
      <Breadcrumb cardName="📱 채널" onReturn={props.onBack} />
      <button class="card-page-back" onClick={props.onBack}>← 홈</button>
      <div class="card-page-head">
        <span class="icon">📱</span>
        <h1>채널</h1>
      </div>
      <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #4 — 인간 친화 채널</div>
      <div class="card-page-oneline">
        Discord·Telegram·Slack·카카오·WhatsApp·Web — 사람 중심 인박스 + 봇 라이프사이클 + 사람별 정책
      </div>

      <nav style="display:flex; gap:4px; margin-bottom:14px;">
        <button class={"link-btn " + (tab() === "inbox" ? "active" : "")} onClick={() => setTab("inbox")}>📥 인박스</button>
        <button class={"link-btn " + (tab() === "person" ? "active" : "")} onClick={() => setTab("person")}>👤 사람</button>
        <button class={"link-btn " + (tab() === "register" ? "active" : "")} onClick={() => setTab("register")}>➕ 채널 등록</button>
        <button class={"link-btn " + (tab() === "routing" ? "active" : "")} onClick={() => setTab("routing")}>🔀 라우팅</button>
        <button class={"link-btn " + (tab() === "moderation" ? "active" : "")} onClick={() => setTab("moderation")}>🛡️ 모더레이션</button>
      </nav>

      <Show when={tab() === "inbox"}>
        <section class="card-section">
          <h3>📥 인박스 — 사양 §3.1 (M-5)</h3>
          <p class="placeholder-note">
            모든 채널·모든 사람의 메시지 통합 타임라인. 사람 클릭 시 그 사람과의 대화. 메시지 클릭 시 💬 메신저로 점프.
          </p>
          <Show when={(inbox() ?? []).length === 0}>
            <div class="card-section-row"><span class="value">인박스 비어있음 (채널 봇 미설정 또는 메시지 없음)</span></div>
          </Show>
          <For each={inbox() ?? []}>
            {(m) => (
              <div class="card-section-row" style="border-bottom:1px solid var(--border); padding:8px 0;">
                <span class="label">{m.sender}</span>
                <span class="value">{m.body.slice(0, 80)}{m.body.length > 80 ? "…" : ""}</span>
              </div>
            )}
          </For>
        </section>
      </Show>

      <Show when={tab() === "person"}>
        <section class="card-section">
          <h3>👤 사람 — 사양 §3.2 (M-6)</h3>
          <p class="placeholder-note">
            한 사람과의 대화 흐름 (어느 에이전트가 답했든). PersonId 식별 — 디스코드·텔레그램·슬랙 같은 사람 통합.
            백엔드 `Person` 테이블 + `GET /v1/gui/channel/people` 신설 필요.
          </p>
        </section>
      </Show>

      <Show when={tab() === "register"}>
        <section class="card-section">
          <h3>➕ 채널 등록 — 사양 §3.3 (M-1 M-2 M-3)</h3>
          <p class="placeholder-note">
            봇 토큰은 자동으로 🗝️ Vault 에 저장 (안티패턴 #3 준수). 본 카드는 등록·연결 테스트만.
          </p>
          <NotifySetup />
        </section>
      </Show>

      <Show when={tab() === "routing"}>
        <section class="card-section">
          <h3>🔀 라우팅 (인간 → 에이전트) — 사양 §3.4 (M-7 V-4)</h3>
          <p class="placeholder-note">
            멘션·키워드·채널별 규칙. "@researcher → ZAL-001", "디스코드 #ops → GCP-001" 등.
            에이전트↔에이전트 라우팅은 💬 메신저 (다른 마스터). 백엔드 신설 필요.
          </p>
        </section>
      </Show>

      <Show when={tab() === "moderation"}>
        <section class="card-section">
          <h3>🛡️ 모더레이션 — 사양 §3.5 (M-10 V-3)</h3>
          <p class="placeholder-note">
            차단 · 신고 · 악성 패턴 감지 · 사람별 일일 메시지 한도. Phase 2.
          </p>
        </section>
      </Show>
    </div>
  );
}
