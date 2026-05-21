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
 const id = await invoke<number | null>("notify_telegram_detect_chat", {
 token: token(),
});
 if (id !== null && id !== undefined) setChatId(String(id));
 else setError("아직 메시지가 도착하지 않았습니다 — 봇에게 /start 보낸 뒤 다시 시도");
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

 <Show when={botUsername()}>
 <div class="form-row" style="margin-top: 10px;">
 <label>{t("notify.chat_id.label")}</label>
 <input
 type="text"
 value={chatId()}
 onInput={(e) => setChatId(e.currentTarget.value)}
 />
 <p class="hint">{t("notify.detect_chat_hint")}</p>
 <div class="row-actions">
 <button type="button" onClick={detect} disabled={busy()}>
 {t("notify.detect_chat")}
 </button>
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
