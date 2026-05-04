import { createResource, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../i18n";

async function checkInitialized(): Promise<boolean> {
  try {
    return await invoke<boolean>("is_initialized");
  } catch {
    return false;
  }
}

export function Onboarding(props: { onReady?: () => void }) {
  const { t } = useI18n();
  const [initialized, { refetch }] = createResource(checkInitialized);

  return (
    <div>
      <div class="card">
        <h2>{t("onboarding.title")}</h2>
        <p class="muted">{t("onboarding.subtitle")}</p>
      </div>

      <div class="card">
        <p class="section-title">{t("onboarding.step1_title")}</p>
        <p>{t("onboarding.step1_desc")}</p>
      </div>

      <div class="card">
        <p class="section-title">{t("onboarding.step2_title")}</p>
        <p>{t("onboarding.step2_desc")}</p>
      </div>

      <div class="card">
        <p class="section-title">{t("onboarding.step3_title")}</p>
        <p>{t("onboarding.step3_desc")}</p>
      </div>

      <div class="card card-soft">
        <Show
          when={initialized() === true}
          fallback={
            <p>
              <span class="badge warn">{t("onboarding.not_ready")}</span>
            </p>
          }
        >
          <p>
            <span class="badge ok">{t("onboarding.ready")}</span>
          </p>
        </Show>
        <div class="row-actions">
          <button
            class="primary"
            type="button"
            onClick={() => {
              refetch();
              if (initialized() === true) props.onReady?.();
            }}
          >
            {t("onboarding.refresh")}
          </button>
        </div>
      </div>
    </div>
  );
}
