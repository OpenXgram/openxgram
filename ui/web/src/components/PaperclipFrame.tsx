// rc.281 — 오케스트레이션 탭 = paperclip 임베드 (iframe).
// 마스터 의도: 오케스트레이션 탭을 누르면 paperclip 화면(자체 사이드메뉴부터 본문 전체)이
// 그대로 임베드되어 나타난다. rc.279 커스텀 Org 에이전트 UI(WorkflowPanel/OrchestrationSection)는
// 이 임베드로 대체. iframe 이 탭 영역(좌측 사이드 + 우측 본문) 전체를 꽉 채운다.
//
// paperclip 은 paperclip.starian.us (tailscale 전용) 로 서빙. 미설정/로딩 실패 대비 fallback 안내.

import { createSignal, Show } from "solid-js";

// iframe src — 상수. 환경에서 override 가능 (VITE_PAPERCLIP_URL).
export const PAPERCLIP_URL: string =
  (import.meta as any).env?.VITE_PAPERCLIP_URL || "https://paperclip.starian.us/";

export function PaperclipFrame() {
  const [failed, setFailed] = createSignal(false);
  const [loaded, setLoaded] = createSignal(false);

  return (
    <div
      style="position:relative; flex:1; min-height:0; width:100%; height:100%; display:flex; flex-direction:column;"
    >
      <Show when={!failed()}>
        <iframe
          src={PAPERCLIP_URL}
          title="paperclip 오케스트레이션"
          style="width:100%; height:100%; flex:1; border:0; display:block; background:var(--surface-1);"
          onLoad={() => setLoaded(true)}
          onError={() => setFailed(true)}
        />
      </Show>

      {/* 로딩 대기 / 연결 실패 fallback 안내 */}
      <Show when={failed() || !loaded()}>
        <div
          style="position:absolute; inset:0; display:flex; flex-direction:column; align-items:center; justify-content:center; gap:10px; text-align:center; padding:24px; pointer-events:none;"
        >
          <Show
            when={failed()}
            fallback={
              <span style="font-size:13px; color:var(--text-3);">
                paperclip 로딩 중…
              </span>
            }
          >
            <strong style="font-size:14px; color:var(--text-1);">
              paperclip 연결 대기
            </strong>
            <span style="font-size:12px; color:var(--text-3); max-width:420px;">
              paperclip 오케스트레이션 화면을 불러오지 못했습니다. tailscale 연결 또는
              인프라 설정을 확인하세요.
            </span>
            <a
              href={PAPERCLIP_URL}
              target="_blank"
              rel="noreferrer"
              style="font-size:12px; color:var(--accent, #3a82f6); pointer-events:auto;"
            >
              {PAPERCLIP_URL} 새 탭에서 열기
            </a>
          </Show>
        </div>
      </Show>
    </div>
  );
}
