-- rc.330 — 방(대화) 단위 설정 저장. (GUI Phase P3)
-- 대화 모델 spec 항목 11(하네스/방설정): 방마다 하네스·역할·오케스트레이션·
-- 시스템 프롬프트·이벤트 규칙을 개별 보관한다. 전역 기본 하네스는
-- identity_settings(key='runtime_config') 를 재사용(전역 ⚙️). 이 테이블은 그 위에
-- 얹는 방-스코프 오버라이드.
--
-- 모든 구조화 필드는 JSON 컬럼으로 유연 보관 (스키마 변경 없이 진화 가능).
--   harness_json        : { runtime, model, perm_mode, exec_mode, cwd,
--                           worktree(bool), isolation(bool), mcp[], vault_scope }
--   roles_json          : { defs:[{name,inst}], assignments:[{role,agent}] }
--   orchestration_json  : [ { label, agent, role/action } ... ] (순서 보존 배열)
--   system_prompt       : 방 전체 시스템 프롬프트 (평문)
--   event_rules_json    : [ { trigger, action } ... ]
--
-- room_key = 대화 식별자 (peer alias, group conv key 등). 동적 — 하드코딩 없음.
-- ⚠️ 저장만(persistence). 턴 시점 강제 적용(enforcement)은 P4.
CREATE TABLE IF NOT EXISTS room_config (
    room_key            TEXT PRIMARY KEY,
    harness_json        TEXT,
    roles_json          TEXT,
    orchestration_json  TEXT,
    system_prompt       TEXT,
    event_rules_json    TEXT,
    updated_at          TEXT
);
