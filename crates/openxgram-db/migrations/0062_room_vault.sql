-- rc.334 — 방(대화) 단위 공유 보안 스코프 (GUI P6: 보안 공유방).
-- 대화 모델 spec 항목 5(보안 공유방): 방 단위 공유 vault — 멤버만 키/파일 복호화·열람, 비멤버 차단.
-- 초대=접근 부여 / 퇴장=회수(멤버십 gate). 암호화+감사로그 필수. 민감키=마스터 승인정책(confirm/mfa).
--
-- ⚠️ 보안 경계(사람 검토 필수):
--  - 이 테이블은 METADATA 만 보관한다. 실제 비밀/파일 본문(secret material)은
--    기존 vault crate(openxgram-vault)가 vault_entries 에 ENCRYPTED-AT-REST 로 보관한다.
--    여기에는 평문 비밀이 절대 들어가지 않는다 (item_key 는 사람이 정한 이름표일 뿐).
--  - 실제 vault 키 형식: "room:<room_key>:<item_key>" — vault_entries.key 로 저장됨(중복 검사 대상).
--  - 멤버십 gate: room_participants(active=1) 만 접근. 비멤버 → 403 (handler 레벨).
--  - 민감(sensitive=1) 항목은 vault ACL policy(confirm/mfa)로 라우팅되어 마스터 승인 없이 복호화 불가.
--
-- 무회귀: 이 테이블에 row 가 없는 방은 보안 스코프 미설정 — 종전 동작 그대로(아무 영향 없음).
-- 동적 스키마: room_key/item_key/alias 는 런타임 값. 하드코딩 자격 없음.

CREATE TABLE IF NOT EXISTS room_vault_item (
    room_key      TEXT NOT NULL,
    item_key      TEXT NOT NULL,              -- 방 안에서의 항목 이름표(사람이 정함). 평문 비밀 아님.
    kind          TEXT NOT NULL DEFAULT 'secret', -- 'secret' | 'file'
    sensitive     INTEGER NOT NULL DEFAULT 0, -- 1=민감 → vault ACL confirm/mfa 정책 경유(마스터 승인).
    vault_key     TEXT NOT NULL,              -- 실제 vault_entries.key ("room:<room_key>:<item_key>"). 본문은 vault crate 가 암호화 보관.
    file_hash     TEXT,                       -- kind='file' 일 때 첨부 아티팩트 해시(attachments 패널 연결). NULL 허용.
    created_by    TEXT NOT NULL,              -- 항목을 추가한 멤버 alias(감사용).
    created_at    TEXT NOT NULL,
    PRIMARY KEY (room_key, item_key)
);

CREATE INDEX IF NOT EXISTS idx_room_vault_item_room
    ON room_vault_item(room_key);

-- 키 회전 필요 marker — 멤버 퇴장 시 그 멤버가 이미 본 비밀의 노출 위험을 기록(자동 회전 X, 사람 결정).
-- spec 항목 5 "(+ 본 키 회전 고려)". 전체 재암호화는 무겁고 위험 → 자동 회전하지 않고 FLAG 만 남긴다.
CREATE TABLE IF NOT EXISTS room_vault_rotation_flag (
    id            TEXT PRIMARY KEY,
    room_key      TEXT NOT NULL,
    reason        TEXT NOT NULL,              -- 예: "eject:<member>" — 퇴장으로 인한 노출.
    flagged_at    TEXT NOT NULL,
    resolved      INTEGER NOT NULL DEFAULT 0  -- 1=마스터가 회전/무시 결정 완료.
);

CREATE INDEX IF NOT EXISTS idx_room_vault_rotation_room
    ON room_vault_rotation_flag(room_key, resolved);
