import { createSignal, Show} from "solid-js";
import { useI18n} from "../i18n";
import { ScheduleView} from "./ScheduleView";
import { ChainView} from "./ChainView";
import { PaymentLimitsView} from "./PaymentLimitsView";
import {
 getBearer,
 getDaemonUrl,
 invoke,
 setBearer,
 setDaemonUrl,
} from "@/api/client";

// Settings 탭 — 운영 설정 모음.
// - Daemon (web GUI 전용 — daemon URL + mcp-token)
// - Schedule (예약 메시지)
// - Chain (메시지 체인 YAML)
// - Payment (일일 결제 한도 + MFA)
// - Locale (한국어/English 토글)
type Section = "daemon" | "schedule" | "chain" | "payment" | "locale";

function DaemonSection() {
 const { t} = useI18n();
 const [url, setUrl] = createSignal(getDaemonUrl());
 const [token, setToken] = createSignal(getBearer() ?? "");
 const [status, setStatus] = createSignal<string>("");

 const save = () => {
 setDaemonUrl(url());
 setBearer(token());
 setStatus(t("settings.daemon.saved"));
};

 const test = async () => {
 setDaemonUrl(url());
 setBearer(token());
 setStatus(t("settings.daemon.testing"));
 try {
 const r = await invoke<unknown>("status");
 setStatus(
 `${t("settings.daemon.ok")}: ${JSON.stringify(r).slice(0, 120)}`,
);
} catch (e) {
 setStatus(`${t("settings.daemon.fail")}: ${(e as Error).message}`);
}
};

 return (
 <div class="card">
 <h3>{t("settings.section.daemon")}</h3>
 <p class="hint">{t("settings.daemon.desc")}</p>
 <div class="form-row">
 <label>{t("settings.daemon.url")}</label>
 <input
 type="text"
 value={url()}
 onInput={(e) => setUrl(e.currentTarget.value)}
 placeholder="/api/gui 또는 http://localhost:47302/v1/gui"
 style={{ width: "100%"}}
 />
 </div>
 <div class="form-row">
 <label>{t("settings.daemon.token")}</label>
 <input
 type="password"
 value={token()}
 onInput={(e) => setToken(e.currentTarget.value)}
 placeholder="Bearer mcp-token"
 style={{ width: "100%"}}
 />
 </div>
 <div class="form-row" style={{ gap: "8px"}}>
 <button type="button" onClick={save}>
 {t("common.save")}
 </button>
 <button type="button" onClick={test}>
 {t("settings.daemon.test")}
 </button>
 </div>
 <Show when={status()}>
 <p class="hint" style={{ "white-space": "pre-wrap"}}>{status()}</p>
 </Show>
 </div>
);
}

function LocaleSection() {
 const { t, setLocale, locale} = useI18n();
 return (
 <div class="card">
 <h3>{t("settings.section.locale")}</h3>
 <p class="hint">{t("settings.locale.desc")}</p>
 <div class="form-row">
 <label>{t("settings.locale.label")}</label>
 <select
 value={locale()}
 onChange={(e) => setLocale(e.currentTarget.value as "ko" | "en")}
 >
 <option value="ko">한국어</option>
 <option value="en">English</option>
 </select>
 </div>
 </div>
);
}

export function SettingsTab() {
 const { t} = useI18n();
 const [section, setSection] = createSignal<Section>("daemon");

 const sections: { id: Section; label: string}[] = [
 { id: "daemon", label: t("settings.section.daemon")},
 { id: "schedule", label: t("settings.section.schedule")},
 { id: "chain", label: t("settings.section.chain")},
 { id: "payment", label: t("settings.section.payment")},
 { id: "locale", label: t("settings.section.locale")},
 ];

 return (
 <div>
 <nav class="subnav" aria-label={t("settings.section.nav")}>
 {sections.map((s) => (
 <button
 type="button"
 class={section() === s.id ? "active" : ""}
 onClick={() => setSection(s.id)}
 >
 {s.label}
 </button>
))}
 </nav>
 <Show when={section() === "daemon"}>
 <DaemonSection />
 </Show>
 <Show when={section() === "schedule"}>
 <ScheduleView />
 </Show>
 <Show when={section() === "chain"}>
 <ChainView />
 </Show>
 <Show when={section() === "payment"}>
 <PaymentLimitsView />
 </Show>
 <Show when={section() === "locale"}>
 <LocaleSection />
 </Show>
 </div>
);
}
