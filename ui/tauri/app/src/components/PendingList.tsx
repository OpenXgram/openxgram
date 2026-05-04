import { createResource, For, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { ask, message } from "@tauri-apps/plugin-dialog";
import { useI18n } from "../i18n";

interface Pending {
  id: string;
  key: string;
  agent: string;
  requested_at: string;
}

async function fetchPending(): Promise<Pending[]> {
  return await invoke<Pending[]>("vault_pending_list");
}

async function approve(id: string): Promise<void> {
  const ok = await ask(`Approve ${id}?`, { kind: "info" });
  if (!ok) return;
  await invoke("vault_pending_approve", { id });
}

async function deny(id: string): Promise<void> {
  const reason = "user-denied"; // TODO: prompt 입력 + Zod 검증
  await invoke("vault_pending_deny", { id, reason });
}

export function PendingList() {
  const { t } = useI18n();
  const [items, { refetch }] = createResource(fetchPending);
  return (
    <div>
      <Show when={!items.loading} fallback={<p>loading…</p>}>
        <Show when={(items() ?? []).length > 0} fallback={<p>{t("pending.empty")}</p>}>
          <ul style="list-style: none; padding: 0;">
            <For each={items()}>
              {(p) => (
                <li
                  style="border: 1px solid #ddd; padding: 8px; margin-bottom: 6px;"
                >
                  <div>
                    <strong>{p.key}</strong>{" "}
                    <small style="color: #666;">{p.agent} · {p.requested_at}</small>
                  </div>
                  <div style="margin-top: 6px;">
                    <button class="primary" onClick={() => approve(p.id).then(refetch)}>
                      {t("pending.approve")}
                    </button>{" "}
                    <button
                      class="danger"
                      onClick={() => deny(p.id).then(refetch)}
                    >
                      {t("pending.deny")}
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

// type-export to silence noUnusedLocals on `message` if untouched in build
export const _Touch = { message };
