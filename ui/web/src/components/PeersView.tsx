import { createResource, createSignal, For, Show} from "solid-js";
import { invoke} from "@/api/client";
import { ask} from "@/api/dialog";
import { useI18n} from "../i18n";

interface Peer {
 alias: string;
 address: string;
 public_key_hex: string;
 machine?: string;
 last_seen?: string;
}

async function fetchPeers(): Promise<Peer[]> {
 return await invoke<Peer[]>("peers_list");
}

function fingerprint(pubkeyHex: string): string {
 // 단순 hex prefix:suffix 표시 — 실제 fingerprint 는 SHA256 해시 first 8 bytes 권장
 const trimmed = pubkeyHex.replace(/^0x/, "");
 if (trimmed.length < 16) return trimmed;
 return `${trimmed.slice(0, 8)}…${trimmed.slice(-8)}`;
}

async function addPeer(form: { alias: string; address: string; pubkey: string; machine?: string}) {
 const fp = fingerprint(form.pubkey);
 const ok = await ask(
 `Fingerprint: ${fp}\nVerify before approve.\n\nProceed?`,
 { kind: "warning"},
);
 if (!ok) return;
 await invoke("peer_add", { ...form});
}

export function PeersView() {
 const { t} = useI18n();
 const [peers, { refetch}] = createResource(fetchPeers);
 const [alias, setAlias] = createSignal("");
 const [address, setAddress] = createSignal("");
 const [pubkey, setPubkey] = createSignal("");
 const [machine, setMachine] = createSignal("");

 const onAdd = async (e: Event) => {
 e.preventDefault();
 if (!alias() || !address() || !pubkey()) return;
 await addPeer({
 alias: alias(),
 address: address(),
 pubkey: pubkey(),
 machine: machine() || undefined,
});
 setAlias("");
 setAddress("");
 setPubkey("");
 setMachine("");
 void refetch();
};

 return (
 <div>
 <form onSubmit={onAdd} style="display: grid; gap: 6px; margin-bottom: 16px;">
 <input placeholder="alias" value={alias()} onInput={(e) => setAlias(e.currentTarget.value)} />
 <input
 placeholder="http://… or nostr://…"
 value={address()}
 onInput={(e) => setAddress(e.currentTarget.value)}
 />
 <input
 placeholder="public_key_hex"
 value={pubkey()}
 onInput={(e) => setPubkey(e.currentTarget.value)}
 />
 <input
 placeholder="machine (optional, ACL whitelist)"
 value={machine()}
 onInput={(e) => setMachine(e.currentTarget.value)}
 />
 <button type="submit">{t("peers.add")}</button>
 </form>

 <p style="color: #b00;">{t("peers.fingerprint_warn")}</p>

 <Show when={!peers.loading} fallback={<p>loading…</p>}>
 <ul style="list-style: none; padding: 0;">
 <For each={peers() ?? []}>
 {(p) => (
 <li style="border: 1px solid #ddd; padding: 8px; margin-bottom: 4px;">
 <strong>{p.alias}</strong>{" "}
 <small style="color: #666;">{fingerprint(p.public_key_hex)}</small>
 <div>{p.address}</div>
 <div style="margin-top: 4px;">
 <button
 type="button"
 style="font-size: 11px; padding: 2px 6px;"
 onClick={async () => {
 const body = prompt(`[디버그] '${p.alias}' 에 비서명 envelope 송신.\nbody (JSON or text):`);
 if (!body) return;
 try {
 const r = await invoke<any>("peer_send_unsigned", { alias: p.alias, body});
 alert("ok: " + JSON.stringify(r).slice(0, 200));
 } catch (e) { alert("실패: " + e);}
 }}
 >🐛 비서명 송신 (debug)</button>
 </div>
 </li>
)}
 </For>
 </ul>
 </Show>
 </div>
);
}
