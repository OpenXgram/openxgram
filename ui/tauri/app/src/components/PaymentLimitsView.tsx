import { createResource, createSignal, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { z } from "zod";
import { useI18n } from "../i18n";

const LimitSchema = z.object({
  daily_limit_usdc_micro: z.number().int().nonnegative(),
});

async function fetchLimit(): Promise<number> {
  const raw = await invoke<unknown>("payment_get_daily_limit");
  return LimitSchema.parse({ daily_limit_usdc_micro: Number(raw) }).daily_limit_usdc_micro;
}

async function setLimit(microUsdc: number): Promise<void> {
  // mfa 정책 — webauthn (PRD-MFA-02) 통합 시 invoke 가 token 요구. 현재는 confirm 만.
  const ok = await ask(
    "MFA re-authentication required. Continue?",
    { kind: "warning" },
  );
  if (!ok) return;
  await invoke("payment_set_daily_limit", { microUsdc });
}

export function PaymentLimitsView() {
  const { t } = useI18n();
  const [current, { refetch }] = createResource(fetchLimit);
  const [draft, setDraft] = createSignal<string>("");

  const onSave = async () => {
    const num = Number(draft());
    if (!Number.isFinite(num) || num < 0) return;
    const micro = Math.floor(num * 1_000_000);
    await setLimit(micro);
    void refetch();
    setDraft("");
  };

  return (
    <div>
      <p style="color: #b00;">{t("payment.mfa_required")}</p>
      <p>
        <strong>{t("payment.daily_limit")}: </strong>
        <Show when={!current.loading} fallback="loading…">
          {((current() ?? 0) / 1_000_000).toFixed(2)} USDC
        </Show>
      </p>
      <div style="display: flex; gap: 6px;">
        <input
          type="number"
          step="0.01"
          min="0"
          value={draft()}
          placeholder="새 한도 (USDC)"
          onInput={(e) => setDraft(e.currentTarget.value)}
        />
        <button onClick={() => void onSave()} disabled={!draft()}>
          {t("common.confirm")}
        </button>
      </div>
    </div>
  );
}
