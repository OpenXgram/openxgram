import { createResource, createSignal, For, Show} from "solid-js";
import { invoke} from "@/api/client";
import { useI18n} from "../i18n";

interface NotifyStatus {
 telegram_configured: boolean;
 discord_configured: boolean;
 discord_webhook_configured: boolean;
}

interface DiscordGuild {
 id: string;
 name: string;
 icon?: string | null;
 owner?: boolean;
}

async function fetchStatus(): Promise<NotifyStatus> {
 return await invoke<NotifyStatus>("notify_status");
}

function TelegramWizard(props: { onSaved: () => void}) {
 const { t} = useI18n();
 const [token, setToken] = createSignal("");
 const [chatId, setChatId] = createSignal("");
 const [botUsername, setBotUsername] = createSignal<string | null>(null);
 const [error, setError] = createSignal<string | null>(null);
 const [busy, setBusy] = createSignal(false);
 const [savedAt, setSavedAt] = createSignal<string | null>(null);

 const validate = async () => {
 setError(null);
 setBusy(true);
 try {
 const res = await invoke<{ bot_username: string}>(
 "notify_telegram_validate",
 { token: token()},
);
 setBotUsername(res.bot_username);
} catch (e) {
 setError(String(e));
} finally {
 setBusy(false);
}
};

 const detect = async () => {
 setError(null);
 setBusy(true);
 try {
 if (!token().trim()) {
 setError("먼저 봇 토큰을 입력하세요 (BotFather 에서 받은 형식 123456:ABC-...).");
 return;
 }
 const id = await invoke<number | null>("notify_telegram_detect_chat", {
 token: token(),
});
 if (id !== null && id !== undefined) {
 setChatId(String(id));
 setError(null);
 } else {
 setError("아직 봇 으로 온 메시지가 없습니다. 1) 텔레그램에서 봇을 검색해 대화 시작 (/start) 2) 메시지 1개 보내기 3) 다시 'chat_id 자동감지' 클릭.");
 }
} catch (e) {
 const msg = String(e);
 // 409 Conflict — 다른 process (Hermes agent 등) 가 이미 같은 봇 token 으로 getUpdates polling 중.
 if (msg.includes("409") || msg.toLowerCase().includes("conflict")) {
 setError(
 "⚠ 봇 토큰 충돌: 다른 process (Hermes 등) 가 이미 이 봇으로 polling 중입니다.\n" +
 "→ chat_id 를 직접 입력하세요 (Telegram 에서 @userinfobot 검색해 본인 chat_id 확인).\n" +
 "→ 또는 다른 polling process 먼저 중지."
 );
 } else {
 setError(msg);
 }
} finally {
 setBusy(false);
}
};

 const saveAndTest = async () => {
 setError(null);
 setBusy(true);
 try {
 const res = await invoke<{ saved_at: string}>("notify_telegram_save", {
 token: token(),
 chat_id: chatId(),
 test_text: "OpenXgram GUI 연결 성공 ",
});
 setSavedAt(res.saved_at);
 props.onSaved();
} catch (e) {
 setError(String(e));
} finally {
 setBusy(false);
}
};

 return (
 <div class="card">
 <h3>{t("notify.section.telegram")}</h3>

 <div class="form-row">
 <label>{t("notify.token.label")}</label>
 <input
 type="text"
 placeholder={t("notify.token.placeholder")}
 value={token()}
 onInput={(e) => setToken(e.currentTarget.value)}
 />
 </div>

 <div class="row-actions">
 <button type="button" onClick={validate} disabled={busy() || !token().trim()}>
 {t("notify.validate")}
 </button>
 <Show when={botUsername()}>
 <span class="badge ok">@{botUsername()}</span>
 </Show>
 </div>

 {/* chat_id 입력·자동감지 — 토큰만 있으면 노출 (validate 없이도) */}
 <Show when={token().trim()}>
 <div class="form-row" style="margin-top: 10px;">
 <label>chat_id (자기 자신 또는 봇이 메시지 받을 곳)</label>
 <input
 type="text"
 placeholder="예: 6565914284 — 자동 감지 권장"
 value={chatId()}
 onInput={(e) => setChatId(e.currentTarget.value)}
 />
 <p class="hint" style="font-size:11px; line-height:1.5;">
 <strong>자동 감지 방법:</strong><br />
 1) Telegram 앱에서 <strong>@{botUsername() || '봇이름'}</strong> 을 검색 후 채팅 시작<br />
 2) <code>/start</code> 또는 아무 메시지 1개 보내기<br />
 3) 아래 <strong>"chat_id 자동감지"</strong> 클릭 → 봇이 받은 메시지의 chat_id 자동 채워짐<br />
 <em>(getUpdates API 가 마지막 update 의 chat_id 반환 — 봇과 대화 시작 후에만 작동)</em>
 </p>
 <div class="row-actions">
 <button type="button" onClick={detect} disabled={busy()}
 style="background:#06c; color:white; padding:8px 14px; border-radius:4px; border:none; cursor:pointer; font-weight:bold;">
 ▶ chat_id 자동감지
 </button>
 <Show when={chatId()}>
 <span class="badge ok" style="margin-left:8px;">✓ {chatId()}</span>
 </Show>
 </div>
 </div>

 <div class="row-actions" style="margin-top: 10px;">
 <button
 class="primary"
 type="button"
 onClick={saveAndTest}
 disabled={busy() || !token().trim() || !chatId().trim()}
 >
 {t("notify.save_and_test")}
 </button>
 </div>
 </Show>

 <Show when={savedAt()}>
 <p class="hint" style="margin-top: 8px;">
 <span class="badge ok">{t("notify.saved_at")}</span> {savedAt()}
 </p>
 </Show>
 <Show when={error()}>
 <p class="error-text">{error()}</p>
 </Show>
 </div>
);
}

function DiscordWizard(props: { onSaved: () => void}) {
 const { t} = useI18n();
 const [token, setToken] = createSignal("");
 const [guildId, setGuildId] = createSignal("");
 const [channelId, setChannelId] = createSignal("");
 const [webhookUrl, setWebhookUrl] = createSignal("");
 const [botLabel, setBotLabel] = createSignal<string | null>(null);
 const [guilds, setGuilds] = createSignal<DiscordGuild[] | null>(null);
 const [error, setError] = createSignal<string | null>(null);
 const [busy, setBusy] = createSignal(false);
 const [savedAt, setSavedAt] = createSignal<string | null>(null);

 const validate = async () => {
 setError(null);
 setBusy(true);
 try {
 const res = await invoke<{ bot_label: string}>(
 "notify_discord_validate",
 { token: token()},
);
 setBotLabel(res.bot_label);
 // 검증되면 곧바로 가입 서버 자동 조회 — guild_id 수동 입력 부담 제거.
 try {
 const gs = await invoke<DiscordGuild[]>("notify_discord_guilds", {
 token: token(),
});
 setGuilds(gs);
 if (gs.length === 1) setGuildId(gs[0].id);
} catch (ge) {
 setGuilds([]);
 console.warn("guilds fetch failed", ge);
}
} catch (e) {
 setError(String(e));
} finally {
 setBusy(false);
}
};

 const saveAndTest = async () => {
 setError(null);
 setBusy(true);
 try {
 const res = await invoke<{ saved_at: string}>("notify_discord_save", {
 token: token(),
 guild_id: guildId() || null,
 channel_id: channelId() || null,
 webhook_url: webhookUrl() || null,
 test_text: webhookUrl() ? "OpenXgram GUI 연결 성공 " : null,
});
 setSavedAt(res.saved_at);
 props.onSaved();
} catch (e) {
 setError(String(e));
} finally {
 setBusy(false);
}
};

 return (
 <div class="card">
 <h3>{t("notify.section.discord")}</h3>

 <p class="hint">
 <a
 href="https://discord.com/developers/applications"
 target="_blank"
 rel="noreferrer"
 >
 Discord Developer Portal
 </a>
 {" → New Application → Bot 탭 → Reset Token → 토큰 복사"}
 </p>

 <div class="form-row">
 <label>{t("notify.token.label")}</label>
 <input
 type="text"
 value={token()}
 onInput={(e) => setToken(e.currentTarget.value)}
 />
 </div>

 <div class="row-actions">
 <button type="button" onClick={validate} disabled={busy() || !token().trim()}>
 {t("notify.validate")}
 </button>
 <Show when={botLabel()}>
 <span class="badge ok">{botLabel()}</span>
 </Show>
 </div>

 <Show when={botLabel()}>
 <div class="form-row" style="margin-top: 10px;">
 <label>{t("notify.guild.label")}</label>
 <Show
 when={guilds() !== null && (guilds() ?? []).length > 0}
 fallback={
 <Show
 when={guilds() !== null}
 fallback={<p class="hint">{t("notify.guild.fetching")}</p>}
 >
 <p class="hint">{t("notify.guild.empty")}</p>
 </Show>
}
 >
 <select
 value={guildId()}
 onChange={(e) => setGuildId(e.currentTarget.value)}
 >
 <option value="">{t("notify.guild.placeholder")}</option>
 <For each={guilds() ?? []}>
 {(g) => (
 <option value={g.id}>
 {g.name} ({g.id})
 </option>
)}
 </For>
 </select>
 </Show>
 </div>

 <div class="form-row">
 <label>{t("notify.channel_id.label")}</label>
 <input
 type="text"
 value={channelId()}
 onInput={(e) => setChannelId(e.currentTarget.value)}
 placeholder="(생략 가능 — 개발자 모드 → 채널 우클릭 → ID 복사)"
 />
 </div>
 <div class="form-row">
 <label>{t("notify.webhook_url.label")}</label>
 <input
 type="text"
 value={webhookUrl()}
 onInput={(e) => setWebhookUrl(e.currentTarget.value)}
 placeholder="(생략 가능 — 채널 설정 → 연동 → 웹훅 만들기)"
 />
 </div>
 <div class="row-actions">
 <button
 class="primary"
 type="button"
 onClick={saveAndTest}
 disabled={busy() || !token().trim()}
 >
 {t("notify.save_and_test")}
 </button>
 </div>
 </Show>

 <Show when={savedAt()}>
 <p class="hint" style="margin-top: 8px;">
 <span class="badge ok">{t("notify.saved_at")}</span> {savedAt()}
 </p>
 </Show>
 <Show when={error()}>
 <p class="error-text">{error()}</p>
 </Show>
 </div>
);
}

export function NotifySetup() {
 const { t} = useI18n();
 const [status, { refetch}] = createResource(fetchStatus);

 return (
 <div>
 <div class="card card-soft">
 <Show
 when={status()?.telegram_configured || status()?.discord_configured}
 fallback={<p class="hint">{t("notify.status.none")}</p>}
 >
 <Show when={status()?.telegram_configured}>
 <span class="badge ok" style="margin-right: 6px;">
 {t("notify.status.telegram_configured")}
 </span>
 </Show>
 <Show when={status()?.discord_configured}>
 <span class="badge ok">
 {t("notify.status.discord_configured")}
 </span>
 </Show>
 </Show>
 </div>

 <TelegramWizard onSaved={() => refetch()} />
 <DiscordWizard onSaved={() => refetch()} />
 </div>
);
}
