import { createResource, createSignal, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "../i18n";

interface Schedule {
  id: string;
  target_kind: string;
  target: string;
  payload: string;
  msg_type: string;
  schedule_kind: string;
  schedule_value: string;
  status: string;
  created_at_kst: number;
  next_due_at_kst: number | null;
  last_error: string | null;
}

interface Stats {
  pending: number;
  sent: number;
  failed: number;
  cancelled: number;
}

async function fetchList(): Promise<Schedule[]> {
  return await invoke<Schedule[]>("schedule_list");
}
async function fetchStats(): Promise<Stats> {
  return await invoke<Stats>("schedule_stats");
}

function ScheduleNewForm(props: { onCreated: () => void }) {
  const { t } = useI18n();
  const [targetKind, setTargetKind] = createSignal<"role" | "platform">("role");
  const [target, setTarget] = createSignal("");
  const [payload, setPayload] = createSignal("");
  const [msgType, setMsgType] = createSignal("info");
  const [scheduleKind, setScheduleKind] = createSignal<"once" | "cron">("once");
  const [scheduleValue, setScheduleValue] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);

  const create = async () => {
    setError(null);
    setBusy(true);
    try {
      await invoke<string>("schedule_create", {
        targetKind: targetKind(),
        target: target(),
        payload: payload(),
        msgType: msgType(),
        scheduleKind: scheduleKind(),
        scheduleValue: scheduleValue(),
      });
      setTarget("");
      setPayload("");
      setScheduleValue("");
      props.onCreated();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="card">
      <p class="section-title">{t("schedule.new")}</p>

      <div class="form-row">
        <label>{t("schedule.target_kind.label")}</label>
        <select
          value={targetKind()}
          onChange={(e) =>
            setTargetKind(e.currentTarget.value as "role" | "platform")
          }
        >
          <option value="role">{t("schedule.target_kind.role")}</option>
          <option value="platform">{t("schedule.target_kind.platform")}</option>
        </select>
      </div>

      <div class="form-row">
        <label>{t("schedule.target.label")}</label>
        <input
          type="text"
          placeholder={
            targetKind() === "role"
              ? t("schedule.target.placeholder.role")
              : t("schedule.target.placeholder.platform")
          }
          value={target()}
          onInput={(e) => setTarget(e.currentTarget.value)}
        />
      </div>

      <div class="form-row">
        <label>{t("schedule.payload.label")}</label>
        <input
          type="text"
          value={payload()}
          onInput={(e) => setPayload(e.currentTarget.value)}
        />
      </div>

      <div class="form-row">
        <label>{t("schedule.msg_type.label")}</label>
        <input
          type="text"
          value={msgType()}
          onInput={(e) => setMsgType(e.currentTarget.value)}
        />
      </div>

      <div class="form-row">
        <label>{t("schedule.schedule_kind.label")}</label>
        <select
          value={scheduleKind()}
          onChange={(e) =>
            setScheduleKind(e.currentTarget.value as "once" | "cron")
          }
        >
          <option value="once">{t("schedule.schedule_kind.once")}</option>
          <option value="cron">{t("schedule.schedule_kind.cron")}</option>
        </select>
      </div>

      <div class="form-row">
        <label>
          {scheduleKind() === "once"
            ? t("schedule.schedule_value.placeholder.once")
            : t("schedule.schedule_value.placeholder.cron")}
        </label>
        <input
          type="text"
          placeholder={
            scheduleKind() === "once"
              ? t("schedule.schedule_value.placeholder.once")
              : t("schedule.schedule_value.placeholder.cron")
          }
          value={scheduleValue()}
          onInput={(e) => setScheduleValue(e.currentTarget.value)}
        />
      </div>

      <div class="row-actions">
        <button
          class="primary"
          type="button"
          onClick={create}
          disabled={busy() || !target().trim() || !payload().trim() || !scheduleValue().trim()}
        >
          {t("schedule.create")}
        </button>
      </div>
      <Show when={error()}>
        <p class="error-text">{error()}</p>
      </Show>
    </div>
  );
}

export function ScheduleView() {
  const { t } = useI18n();
  const [list, { refetch }] = createResource(fetchList);
  const [stats, { refetch: refetchStats }] = createResource(fetchStats);

  const cancel = async (id: string) => {
    try {
      await invoke("schedule_cancel", { id });
      refetch();
      refetchStats();
    } catch (e) {
      alert(String(e));
    }
  };

  return (
    <div>
      <div class="stats-grid">
        <div class="stat-card">
          <span class="stat-value">{stats()?.pending ?? 0}</span>
          <span class="stat-label">{t("schedule.stats.pending")}</span>
        </div>
        <div class="stat-card">
          <span class="stat-value">{stats()?.sent ?? 0}</span>
          <span class="stat-label">{t("schedule.stats.sent")}</span>
        </div>
        <div class="stat-card">
          <span class="stat-value">{stats()?.failed ?? 0}</span>
          <span class="stat-label">{t("schedule.stats.failed")}</span>
        </div>
        <div class="stat-card">
          <span class="stat-value">{stats()?.cancelled ?? 0}</span>
          <span class="stat-label">{t("schedule.stats.cancelled")}</span>
        </div>
      </div>

      <ScheduleNewForm
        onCreated={() => {
          refetch();
          refetchStats();
        }}
      />

      <Show when={!list.loading} fallback={<p>{t("common.loading")}</p>}>
        <Show
          when={(list() ?? []).length > 0}
          fallback={<p class="hint">{t("schedule.list_empty")}</p>}
        >
          <ul class="plain-list">
            <For each={list()}>
              {(s) => (
                <li>
                  <div>
                    <strong>{s.target_kind}</strong>:{s.target}{" "}
                    <span class="badge">{s.status}</span>
                  </div>
                  <div class="hint" style="margin-top: 4px;">
                    {s.schedule_kind} = <code>{s.schedule_value}</code>
                  </div>
                  <div style="margin-top: 4px;">{s.payload}</div>
                  <Show when={s.last_error}>
                    <p class="error-text">
                      {t("schedule.last_error")}: {s.last_error}
                    </p>
                  </Show>
                  <div class="row-actions" style="margin-top: 6px;">
                    <button
                      class="danger"
                      type="button"
                      onClick={() => cancel(s.id)}
                      disabled={s.status !== "pending"}
                    >
                      {t("schedule.cancel")}
                    </button>
                  </div>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </Show>
    </div>
  );
}
