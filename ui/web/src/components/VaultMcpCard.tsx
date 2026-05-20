import { createSignal, Show } from "solid-js";
import { VaultView } from "./VaultView";
import { Breadcrumb } from "./Breadcrumb";

// UI-VAULT-MCP-SPEC v1.0 §3 — 🗝️ 도구·Vault·MCP 카드 (PRD §0 #8).
// 4 탭: 시크릿 · MCP 서버 · 도구 카탈로그 · 감사 로그.

type Tab = "secret" | "mcp" | "tool" | "audit";

export function VaultMcpCard(props: { onBack: () => void }) {
  const [tab, setTab] = createSignal<Tab>("secret");

  return (
    <div class="card-page">
      <Breadcrumb cardName="🗝️ 도구·Vault·MCP" onReturn={props.onBack} />
      <button class="card-page-back" onClick={props.onBack}>← 홈</button>
      <div class="card-page-head">
        <span class="icon">🗝️</span>
        <h1>도구·Vault·MCP</h1>
      </div>
      <div class="card-page-prd">PRD-OpenXgram v1.4 §0 #8 — 도구·Vault·MCP</div>
      <div class="card-page-oneline">
        시크릿 저장 (ChaCha20-Poly1305) · MCP 서버 등록 · 도구 카탈로그 · default-deny 정책 · 감사 로그
      </div>

      <nav style="display:flex; gap:4px; margin-bottom:14px;">
        <button class={"link-btn " + (tab() === "secret" ? "active" : "")} onClick={() => setTab("secret")}>🔐 시크릿</button>
        <button class={"link-btn " + (tab() === "mcp" ? "active" : "")} onClick={() => setTab("mcp")}>🔌 MCP 서버</button>
        <button class={"link-btn " + (tab() === "tool" ? "active" : "")} onClick={() => setTab("tool")}>🛠️ 도구 카탈로그</button>
        <button class={"link-btn " + (tab() === "audit" ? "active" : "")} onClick={() => setTab("audit")}>📋 감사 로그</button>
      </nav>

      <Show when={tab() === "secret"}>
        <section class="card-section">
          <h3>🔐 시크릿 (Secret)</h3>
          <p class="placeholder-note">
            기존 VaultView 통합. 사양 §3.1 — API 키·봇 토큰·DB 자격·webhook 등. `vault://&lt;path&gt;` 핸들로 다른 카드에서 참조만.
          </p>
          <VaultView />
        </section>
      </Show>

      <Show when={tab() === "mcp"}>
        <section class="card-section">
          <h3>🔌 MCP 서버 — 사양 §3.2</h3>
          <p class="placeholder-note">
            등록된 MCP 서버 (filesystem·github·postgres·custom) · 헬스체크 · 재연결.
            백엔드: `xgram mcp-install --scope user` 로 `~/.claude.json` 에 추가 → daemon 측 API 신설 필요.
          </p>
          <div class="card-section-row">
            <span class="label">openxgram (자체)</span>
            <span class="value">/usr/local/bin/xgram mcp-serve · stdio · ✓ user scope</span>
          </div>
          <button class="link-btn">+ MCP 서버 등록</button>
          <button class="link-btn">전체 헬스체크</button>
        </section>
      </Show>

      <Show when={tab() === "tool"}>
        <section class="card-section">
          <h3>🛠️ 도구 카탈로그 — 사양 §3.3</h3>
          <div class="card-section-row">
            <span class="label">기본 정책</span>
            <span class="value">default-deny (안티패턴 #3 준수)</span>
          </div>
          <p class="placeholder-note">
            filesystem · shell · net · payment · llm-call 등. 각 도구의 ACL · auto/confirm/mfa 정책.
            세션별 grant 는 💬 메신저 탭 12에서 참조만.
          </p>
        </section>
      </Show>

      <Show when={tab() === "audit"}>
        <section class="card-section">
          <h3>📋 Vault 감사 로그 — 사양 §3.4</h3>
          <p class="placeholder-note">
            시크릿 접근·등록·로테이션 영구 기록 (M-11). 신원 카드의 인증 감사와 별도.
            백엔드 `GET /v1/gui/vault/audit` 신설 필요.
          </p>
        </section>
      </Show>
    </div>
  );
}
