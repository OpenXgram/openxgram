-- rc.333 — 방(대화) 동적 멤버십 (GUI P5).
-- 대화 모델 spec 항목 1(대화 단위=방={참가자목록+메시지스레드}) + 항목 4(동적 멤버십: 초대/내보내기).
--
-- 방의 활성 참가자 목록을 영속한다. 초대=row 추가(active=1)+맥락 인계, 내보내기=active=0(제거+수신 중단).
-- room_key 는 handle_task 의 conv_key(수신자 bare-alias 스레드 키)와 동일 단위 — 방의 누적 메시지
-- 스레드는 acp_messages(conv_key=room_key)에 이미 쌓인다. 이 테이블은 "누가 그 방의 멤버인가"만 보관.
--
-- 무회귀(중요): 1:1 방은 이 테이블에 row 가 없다 → 멤버십 gate 가 통과(=종전 단일-alias 동작 그대로).
-- 그룹 방은 row 가 생기는 순간부터 active 참가자만 전달/턴 대상이 된다.
--
-- 사람(고권한 참가자, spec 항목 9)은 role='human' 으로 표기 가능 — 항상 암묵적 high-privilege.
-- 동적 스키마: alias/role 은 런타임 값, 하드코딩 자격 없음.

CREATE TABLE IF NOT EXISTS room_participants (
    room_key      TEXT NOT NULL,
    member_alias  TEXT NOT NULL,
    role          TEXT,                       -- 자유 텍스트 역할 라벨(참가자/관찰자/진행자/human 등). NULL 허용.
    joined_at     TEXT NOT NULL,
    active        INTEGER NOT NULL DEFAULT 1,  -- 1=활성 멤버, 0=내보내짐(이력 보존).
    PRIMARY KEY (room_key, member_alias)
);

CREATE INDEX IF NOT EXISTS idx_room_participants_active
    ON room_participants(room_key, active);
