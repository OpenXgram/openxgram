import { PendingList } from "./PendingList";
import { VaultRevealView } from "./VaultRevealView";
import { useI18n } from "../i18n";

/// Vault 통합 — Pending + Reveal 한 화면.
export function VaultView() {
  const { t } = useI18n();
  return (
    <div>
      <div class="card">
        <h2>{t("tab.pending")}</h2>
        <PendingList />
      </div>
      <div class="card">
        <h2>{t("tab.vault_reveal")}</h2>
        <VaultRevealView />
      </div>
    </div>
  );
}
