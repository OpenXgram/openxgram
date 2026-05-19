import { createSignal, Show } from "solid-js";
import { useI18n } from "../i18n";
import { ScheduleView } from "./ScheduleView";
import { ChainView } from "./ChainView";
import { PaymentLimitsView } from "./PaymentLimitsView";

// Settings 탭 — 운영 설정 모음.
//   - Schedule (예약 메시지)
//   - Chain    (메시지 체인 YAML)
//   - Payment  (일일 결제 한도 + MFA)
//   - Locale   (한국어/English 토글)
type Section = "schedule" | "chain" | "payment" | "locale";

function LocaleSection() {
  const { t, setLocale, locale } = useI18n();
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
  const { t } = useI18n();
  const [section, setSection] = createSignal<Section>("schedule");

  const sections: { id: Section; label: string }[] = [
    { id: "schedule", label: t("settings.section.schedule") },
    { id: "chain", label: t("settings.section.chain") },
    { id: "payment", label: t("settings.section.payment") },
    { id: "locale", label: t("settings.section.locale") },
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
