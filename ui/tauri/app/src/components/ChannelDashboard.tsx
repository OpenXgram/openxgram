import { createResource, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../i18n";

interface Adapter {
  platform: string;
  configured: boolean;
  note: string | null;
}

interface Status {
  adapters: Adapter[];
  peer_count: number;
  schedule_pending: number;
}

interface RecentMessage {
  source: string;
  summary: string;
  timestamp_kst: number;
}

async function fetchStatus(): Promise<Status> {
  return await invoke<Status>("channel_status");
}

async function fetchRecent(): Promise<RecentMessage[]> {
  return await invoke<RecentMessage[]>("channel_recent_messages", { limit: 20 });
}

function formatKstEpoch(epoch: number): string {
  if (!epoch) return "-";
  const d = new Date(epoch * 1000);
  // KST = UTC+9 — Tauri 데몬이 KST epoch 으로 저장하므로 그대로 표시.
  // toISOString 은 UTC 변환되므로 수동 포맷.
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())} KST`;
}

export function ChannelDashboard() {
  const { t } = useI18n();
  const [status, { refetch: refetchStatus }] = createResource(fetchStatus);
  const [recent, { refetch: refetchRecent }] = createResource(fetchRecent);

  const refresh = () => {
    refetchStatus();
    refetchRecent();
  };

  return (
    <div>
      <div class="card">
        <h2>{t("channel.title")}</h2>
        <div class="row-actions">
          <button type="button" onClick={refresh}>
            {t("channel.refresh")}
          </button>
        </div>
      </div>

      <div class="card">
        <p class="section-title">{t("channel.adapters")}</p>
        <Show when={status()} fallback={<p>{t("common.loading")}</p>}>
          <ul class="plain-list">
            <For each={status()!.adapters}>
              {(a) => (
                <li>
                  <strong>{a.platform}</strong>{" "}
                  <span class={a.configured ? "badge ok" : "badge"}>
                    {a.configured
                      ? t("channel.connected")
                      : t("channel.disconnected")}
                  </span>
                  <Show when={a.note}>
                    <div class="hint">{a.note}</div>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>

      <div class="stats-grid">
        <div class="stat-card">
          <span class="stat-value">{status()?.peer_count ?? 0}</span>
          <span class="stat-label">{t("channel.peers")}</span>
        </div>
        <div class="stat-card">
          <span class="stat-value">{status()?.schedule_pending ?? 0}</span>
          <span class="stat-label">{t("channel.schedule_pending")}</span>
        </div>
      </div>

      <div class="card">
        <p class="section-title">{t("channel.recent")}</p>
        <Show when={!recent.loading} fallback={<p>{t("common.loading")}</p>}>
          <Show
            when={(recent() ?? []).length > 0}
            fallback={<p class="hint">{t("channel.recent_empty")}</p>}
          >
            <ul class="plain-list">
              <For each={recent()}>
                {(m) => (
                  <li>
                    <div>
                      <strong>{m.source}</strong>{" "}
                      <span class="hint">
                        {formatKstEpoch(m.timestamp_kst)}
                      </span>
                    </div>
                    <div>{m.summary}</div>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
      </div>
    </div>
  );
}
