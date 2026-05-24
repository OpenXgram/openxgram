import { createEffect, createResource, createSignal, Show} from "solid-js";
import { invoke} from "@/api/client";
import { isUnlocked, lock} from "@/api/auth";
import { I18nProvider, useI18n} from "./i18n";
import { Onboarding} from "./components/Onboarding";
import { ChatTab} from "./components/ChatTab";
import { MemoryTab} from "./components/MemoryTab";
import { NetworkTab} from "./components/NetworkTab";
import { SettingsTab} from "./components/SettingsTab";
import { LoginView} from "./components/LoginView";
import { HomeDashboard, type CardId} from "./components/HomeDashboard";
import { IdentityCard} from "./components/IdentityCard";
import { VaultMcpCard} from "./components/VaultMcpCard";
import { ChannelCard} from "./components/ChannelCard";
import { MemoryCard} from "./components/MemoryCard";
import { AutonomyCard} from "./components/AutonomyCard";
import { ExternalAgentCard} from "./components/ExternalAgentCard";
import { OpsCard} from "./components/OpsCard";
import { ApprovalQueueBell} from "./components/ApprovalQueueBell";
import { GlobalSearchModal} from "./components/GlobalSearchModal";

// PRD-OpenXgram v1.4 §0 + UI-CARDS-IDENTITY v1.1: 홈 대시보드 = 8 카드 (4 가치 + 4 토대).
// unlock 후 첫 화면 = HomeDashboard. 카드 클릭 시 해당 카드 전용 페이지 진입.
type Tab =
 | "onboarding"
 | "home"
 | "chat"
 | "memory"
 | "network"
 | "settings"
 | "card-identity"
 | "card-vault"
 | "card-channel"
 | "card-memory"
 | "card-autonomy"
 | "card-external"
 | "card-ops";

async function checkInitialized(): Promise<boolean> {
 try {
 return await invoke<boolean>("is_initialized");
} catch {
 return false;
}
}

async function checkUnlocked(): Promise<boolean> {
 return await isUnlocked();
}

function AppInner() {
 const { t, setLocale, locale} = useI18n();
 const [authed, { refetch: refetchAuth}] = createResource(checkUnlocked);
 const [initialized] = createResource(
 () => authed() === true,
 async (ok) => (ok ? await checkInitialized() : false),
);
 // 기본 = home (8 카드 대시보드). Onboarding 은 daemon 이 명시적으로 false 일 때만.
 const [tab, setTab] = createSignal<Tab>("home");

 // rc.92 — 버전 변경 감지 + 팝업 + 자동 새로고침.
 // 첫 응답을 baseline 으로 저장 → 30초 폴링 → release 다르면 changelog 와 함께 팝업.
 const [updateInfo, setUpdateInfo] = createSignal<{from: string; to: string; title?: string; body?: string} | null>(null);
 (() => {
 let baseline: string | null = null;
 const poll = async () => {
 try {
 const v = await invoke<any>("version_info");
 const cur = v?.release as string | undefined;
 if (!cur) return;
 if (baseline === null) { baseline = cur; return; }
 if (cur !== baseline) {
 setUpdateInfo({from: baseline, to: cur, title: v?.changelog_latest_title, body: v?.changelog_latest_body});
 }
 } catch { /* daemon 잠시 down — 다음 polling */ }
 };
 // 초기 + 30초마다
 setTimeout(poll, 1000);
 setInterval(poll, 30000);
 })();

 // initialized 가 false 로 확정되면 onboarding 강제. true 면 home 으로 복귀.
 createEffect(() => {
 const init = initialized();
 if (init === false && tab() !== "onboarding") setTab("onboarding");
 if (init === true && tab() === "onboarding") setTab("home");
});

 // 카드 클릭 → 카드 전용 페이지. 메신저만 기존 ChatTab 전체 화면 사용 (실시간 + 시각화 무대).
 function openCard(id: CardId) {
 switch (id) {
 case "messenger": setTab("chat"); break;
 case "memory": setTab("card-memory"); break;
 case "channel": setTab("card-channel"); break;
 case "autonomy": setTab("card-autonomy"); break;
 case "vault": setTab("card-vault"); break;
 case "external": setTab("card-external"); break;
 case "identity": setTab("card-identity"); break;
 case "ops": setTab("card-ops"); break;
 default: setTab("home"); break;
}
}

 // 옛 tabs (chat/memory/network/settings) 제거됨 — 8 카드가 진짜 진입로.
 // 카드 안에서 추가 진입 (메신저 → 설정 등) 은 onJumpToSettings 같은 prop 으로 직접 카드 ID 로 setTab.

 const onLogout = async () => {
 lock();
 refetchAuth();
};

 return (
 <div class="app-shell">
 <header class="app-header">
 <h1 class="app-title">
 OpenXgram <span class="app-version" title={`build ${__BUILD_TIME__}`}>v{__APP_VERSION__}</span>
 </h1>
 <div style={{ display: "flex", "align-items": "center", gap: "8px"}}>
 <select
 value={locale()}
 onChange={(e) => setLocale(e.currentTarget.value as "ko" | "en")}
 aria-label="Locale"
 >
 <option value="ko">한국어</option>
 <option value="en">English</option>
 </select>
 <Show when={authed() === true}>
 <Show when={tab() !== "onboarding" && tab() !== "home"}>
 <button
 type="button"
 onClick={() => setTab("home")}
 title="홈 대시보드 — 8 카드"
 style="background:transparent; border:1px solid var(--border); border-radius:4px; padding:4px 12px; cursor:pointer; color:var(--text-1); font-size:13px; white-space:nowrap;"
 >
 홈
 </button>
 </Show>
 <SearchButton />
 <ApprovalQueueBell />
 <button type="button" onClick={onLogout} style="white-space:nowrap;">
 {t("auth.logout")}
 </button>
 </Show>
 </div>
 </header>

 {/* 인증 화면 — Bearer 없음/만료 */}
 <Show when={authed.loading}>
 <main>
 <p class="hint">{t("common.loading")}</p>
 </main>
 </Show>
 <Show when={!authed.loading && authed() !== true}>
 <main>
 <LoginView onUnlock={() => refetchAuth()} />
 </main>
 </Show>

 {/* 메인 GUI — 인증된 사용자만 */}
 <Show when={authed() === true}>
 <main>
 <Show when={tab() === "onboarding"}>
 <Onboarding onReady={() => setTab("home")} />
 </Show>
 <Show when={tab() === "home"}>
 <HomeDashboard onOpen={openCard} />
 </Show>
 <Show when={tab() === "chat"}>
 <ChatTab onJumpToSettings={() => setTab("settings")} />
 </Show>
 <Show when={tab() === "memory"}>
 <MemoryTab />
 </Show>
 <Show when={tab() === "network"}>
 <NetworkTab />
 </Show>
 <Show when={tab() === "settings"}>
 <SettingsTab />
 </Show>
 <Show when={tab() === "card-identity"}>
 <IdentityCard onBack={() => setTab("home")} />
 </Show>
 <Show when={tab() === "card-vault"}>
 <VaultMcpCard onBack={() => setTab("home")} />
 </Show>
 <Show when={tab() === "card-channel"}>
 <ChannelCard onBack={() => setTab("home")} />
 </Show>
 <Show when={tab() === "card-memory"}>
 <MemoryCard onBack={() => setTab("home")} />
 </Show>
 <Show when={tab() === "card-autonomy"}>
 <AutonomyCard onBack={() => setTab("home")} />
 </Show>
 <Show when={tab() === "card-external"}>
 <ExternalAgentCard onBack={() => setTab("home")} />
 </Show>
 <Show when={tab() === "card-ops"}>
 <OpsCard onBack={() => setTab("home")} />
 </Show>
 </main>
 </Show>

 {/* rc.92 — 버전 업데이트 팝업 (자동 새로고침) */}
 <Show when={updateInfo()}>
 <div style="position:fixed; inset:0; background:rgba(0,0,0,0.6); z-index:9999; display:flex; align-items:center; justify-content:center;">
 <div style="background:var(--surface); border:1px solid var(--border); border-radius:8px; padding:20px; max-width:600px; max-height:80vh; overflow:auto; box-shadow:0 10px 40px rgba(0,0,0,0.5);">
 <h2 style="margin:0 0 8px;">🚀 OpenXgram 업데이트</h2>
 <p style="margin:4px 0; color:var(--text-2);">
 <code>{updateInfo()!.from}</code> → <strong style="color:var(--accent);"><code>{updateInfo()!.to}</code></strong>
 </p>
 <Show when={updateInfo()!.title}>
 <h3 style="margin:12px 0 6px; font-size:14px;">{updateInfo()!.title}</h3>
 </Show>
 <Show when={updateInfo()!.body}>
 <pre style="background:var(--surface-2); padding:10px; border-radius:4px; font-size:12px; white-space:pre-wrap; line-height:1.5; max-height:300px; overflow:auto;">{updateInfo()!.body}</pre>
 </Show>
 <div style="display:flex; gap:8px; margin-top:14px; justify-content:flex-end;">
 <button type="button" onClick={() => setUpdateInfo(null)}
 style="padding:8px 14px; background:var(--surface-2); color:var(--text-1); border:1px solid var(--border); border-radius:4px;">
 나중에
 </button>
 <button type="button" onClick={() => { window.location.reload(); }}
 style="padding:8px 18px; background:#238636; color:white; border:none; border-radius:4px; font-weight:bold;">
 ▶ 지금 새로고침
 </button>
 </div>
 </div>
 </div>
 </Show>
 </div>
);
}

function SearchButton() {
 const [open, setOpen] = createSignal(false);
 return (
 <>
 <button
 type="button"
 onClick={() => setOpen(true)}
 title="글로벌 검색 (N4) — 피어/지식/감사 통합"
 style="background:transparent; border:1px solid var(--border); border-radius:4px; padding:4px 10px; cursor:pointer; color:var(--text-1); font-size:13px; white-space:nowrap;"
 >
 검색
 </button>
 <Show when={open()}>
 <GlobalSearchModal onClose={() => setOpen(false)} />
 </Show>
 </>
);
}

export function App() {
 return (
 <I18nProvider>
 <AppInner />
 </I18nProvider>
);
}
