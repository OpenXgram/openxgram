-- rc.335 — Phase 4b: "에이전트 추가" (남의 에이전트 사용) 상호 동의 handshake + 소유자 가격 + 격리 실행.
--
-- 4a(머신 추가)는 한쪽 등록(전권). 4b 는 다른 사람의 에이전트를 **상거래** 경로로 사용:
--   요청자(requester)가 사용 요청 → 그 에이전트의 소유자(owner)가 수락 AND 가격 책정 →
--   수락 후에만 요청자가 격리 워크트리에서 구동 가능 → 사용량은 소유자 가격으로 과금(원장).
--
-- 전달(delivery)은 새 transport 를 만들지 않는다 — 기존 peer envelope(peer_send /
--   run_peer_send_with_conv)를 재사용해 상대 머신 데몬으로 서명된 envelope 를 보낸다.
--   본문 prefix `[AGENT_ADD_REQUEST]` 로 수신측이 분류한다.
--
-- 가격(price)은 소유자가 accept 시점에 책정한다(amount + unit + currency, 기본 USDC).
--   실제 USDC 정산은 기존 결제 인프라(openxgram-payment / wallet_ledger)의 책임이며,
--   여기서는 charge 이벤트를 원장(agent_add_usage)에 기록만 한다(가짜 영수증 금지).
--
-- 격리(sandbox)는 기존 git worktree 격리(a2a_send endpoint="worktree" / handle_task)를
--   재사용한다. friend_isolated=1 인 추가-에이전트는 메인 트리가 아닌 fresh worktree 에서 구동.
--   ⚠️ OS 컨테이너 격리(진짜 샌드박스)는 미구현 — worktree 격리(파일시스템 분리)만 제공.

-- ── handshake 요청 테이블 ──────────────────────────────────────────────
-- 한 row = 한 건의 "남의 에이전트 사용" 요청. 양쪽(요청자/소유자)이 같은 row 를 본다.
--   요청자 머신: 자기가 만든 outgoing 요청(상태 추적).
--   소유자 머신: peer envelope 로 도착한 incoming 요청(수락/거절 대상).
CREATE TABLE IF NOT EXISTS agent_add_request (
    id                TEXT PRIMARY KEY,            -- uuid (양쪽 머신 공통 id — envelope 로 전달)
    requester         TEXT NOT NULL,               -- 요청자 alias (나)
    requester_machine TEXT,                        -- 요청자 머신 라벨/주소
    target_agent      TEXT NOT NULL,               -- 사용 요청 대상 에이전트 alias
    target_owner      TEXT,                         -- 그 에이전트의 소유자 alias (소유자 머신 primary)
    target_machine    TEXT,                         -- 대상 에이전트 머신 라벨/주소
    status            TEXT NOT NULL DEFAULT 'pending', -- pending|accepted|rejected|revoked
    price_amount      REAL,                         -- 소유자 책정 가격(수락 시) — NULL=미책정
    price_unit        TEXT,                         -- per_call|per_token|subscription|flat
    currency          TEXT NOT NULL DEFAULT 'USDC', -- 기본 USDC (on Base)
    terms             TEXT,                         -- 소유자가 명시한 이용 조건(자유 텍스트)
    direction         TEXT NOT NULL DEFAULT 'incoming', -- incoming(소유자측) | outgoing(요청자측)
    created_at_kst    TEXT NOT NULL,               -- KST 타임스탬프 (절대 규칙 #4)
    decided_at_kst    TEXT                          -- 수락/거절/취소 시각 (KST)
);
CREATE INDEX IF NOT EXISTS idx_agent_add_req_status ON agent_add_request(status);
CREATE INDEX IF NOT EXISTS idx_agent_add_req_target ON agent_add_request(target_agent);
CREATE INDEX IF NOT EXISTS idx_agent_add_req_requester ON agent_add_request(requester);

-- ── 사용량/과금 원장 ───────────────────────────────────────────────────
-- accepted 요청의 에이전트를 요청자가 구동(turn)할 때마다 1 row. 소유자 가격으로 charge 기록.
--   ⚠️ 실제 USDC 정산은 기존 payment 인프라 책임 — 여기서는 charge 이벤트 ledger 만(날조 금지).
--   settled=0(미정산) 으로 시작. 기존 payment 인프라가 정산 시 settled=1 + tx_ref 로 채움.
CREATE TABLE IF NOT EXISTS agent_add_usage (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id        TEXT NOT NULL,               -- agent_add_request.id
    requester         TEXT NOT NULL,               -- 과금 대상(사용자)
    target_agent      TEXT NOT NULL,
    target_owner      TEXT,                         -- 수익 귀속(소유자)
    occurred_at_kst   TEXT NOT NULL,
    price_amount      REAL,                         -- 적용된 단가(요청의 price_amount 스냅샷)
    price_unit        TEXT,
    currency          TEXT,
    units             REAL NOT NULL DEFAULT 1,      -- per_call=1, per_token=토큰수 등
    charge_amount     REAL,                         -- price_amount * units (계산된 청구액)
    settled           INTEGER NOT NULL DEFAULT 0,   -- 0=미정산(기존 payment 인프라 책임), 1=정산됨
    tx_ref            TEXT,                          -- 정산 tx hash / internal ledger ref
    note              TEXT
);
CREATE INDEX IF NOT EXISTS idx_agent_add_usage_req ON agent_add_usage(request_id);
CREATE INDEX IF NOT EXISTS idx_agent_add_usage_settled ON agent_add_usage(settled);
