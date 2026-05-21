import { createResource, onCleanup, onMount, Show} from "solid-js";
import { invoke} from "@/api/client";

interface Peer {
 alias?: string;
 address?: string;
}

interface NotifyStatus {
 discord_configured: boolean;
}

async function fetchPeers(): Promise<Peer[]> {
 try {
 return await invoke<Peer[]>("peers_list");
} catch {
 return [];
}
}

async function fetchNotify(): Promise<NotifyStatus | null> {
 try {
 return await invoke<NotifyStatus>("notify_status");
} catch {
 return null;
}
}

// 첫 사용자 시나리오 가이드 — ChatTab 상단에 표시되는 카드.
// peers 가 있으면 (= 이미 다른 세션과 연결된 상태) 자동 숨김.
//
// 시나리오: 같은 폴더에서 claude/codex 두 에이전트가 OpenXgram 으로
// 서로 대화 + 그 대화가 Discord 채널에 미러링 + Discord 에서 사용자가
// 끼어들기 (사용자가 "B로 하고 싶은데" 라고 명시한 5분 시나리오).
export function PairingGuide(props: { onJumpToSettings: () => void}) {
 const [peers, { refetch: refetchPeers}] = createResource(fetchPeers);
 const [notify, { refetch: refetchNotify}] = createResource(fetchNotify);

 // Settings 에서 Discord 저장 후 ChatTab 으로 돌아왔을 때 stale 한 "(아직)"
 // 표시 방지: (1) 탭 가시화 시 (Page Visibility API) (2) 윈도우 focus
 // (3) 30초 주기 fallback. 모두 cleanup 한다.
 const refreshAll = () => {
 refetchPeers();
 refetchNotify();
};
 const onVisibility = () => {
 if (document.visibilityState === "visible") refreshAll();
};
 onMount(() => {
 document.addEventListener("visibilitychange", onVisibility);
 window.addEventListener("focus", refreshAll);
 const id = window.setInterval(refreshAll, 30_000);
 onCleanup(() => {
 document.removeEventListener("visibilitychange", onVisibility);
 window.removeEventListener("focus", refreshAll);
 window.clearInterval(id);
});
});

 const isEmpty = () => {
 const list = peers();
 return Array.isArray(list) && list.length === 0;
};

 return (
 <Show when={!peers.loading && isEmpty()}>
 <div class="card">
 <h3 style="margin-top:0;">이 머신은 OpenXgram 가입 완료 </h3>
 <p class="hint" style="margin-bottom:14px;">
 5분 시나리오: 같은 프로젝트 폴더의 Claude/Codex 두 에이전트가
 OpenXgram 으로 서로 대화 → Discord 채널에 자동 미러 → Discord 에서
 내가 끼어들기.
 </p>

 <ol style="line-height:1.9; padding-left:20px;">
 <li>
 <strong>다른 폴더에서 <code>claude</code> 또는 <code>codex</code> 실행</strong>
 <br />
 <span class="hint">
 `xgram init` 으로 user scope MCP 등록·SessionStart hook 자동 완료.
 어떤 폴더에서 LLM 켜도 openxgram 27 도구 자동 로드.
 </span>
 </li>
 <li>
 <strong>각 세션 안에서 한 줄: "register_subagent 호출"</strong>
 <br />
 <span class="hint">
 role 만 알려주면 alias 발급 (예: claude-abc123) + peer 등록.
 완료되면 이 화면 친구 목록에 두 세션이 보임.
 </span>
 </li>
 <li>
 <strong>
 <a
 href="#"
 onClick={(e) => {
 e.preventDefault();
 props.onJumpToSettings();
}}
 >
 Discord 봇 토큰 등록
 </a>{" "}
 {notify()?.discord_configured ? "" : "(아직)"}
 </strong>
 <br />
 <span class="hint">
 Settings → 알림 채널 → Discord 카드. 토큰 저장 후 daemon
 재시작 시 인바운드 listener 자동 가동 (Discord → recv_messages).
 </span>
 </li>
 <li>
 <strong>각 세션에서 "create_project_category 호출"</strong>
 <br />
 <span class="hint">
 Discord 서버에 이 프로젝트용 카테고리 + 채널 자동 생성.
 이후 peer_send 가 자동으로 Discord 미러링 (hook 설정 시).
 </span>
 </li>
 </ol>
 </div>
 </Show>
);
}
