-- rc.332 — 오케스트레이션 RUN 상태 (GUI P4c).
-- 대화 모델 spec 항목 11(오케스트레이션·진행자): 방의 orchestration_json(순서 보존 단계 배열)을
-- 데몬이 순서대로 실제 실행한다. 이 테이블은 그 실행(run)의 진행 상태를 영속해
-- UI 가 "현재 단계 N/총"·각 단계 결과를 표시할 수 있게 한다.
--
-- room_config.orchestration_json = "무엇을 어떤 순서로"(설계, P3 저장).
-- orchestration_run            = "지금 어디까지 실행됐나"(실행 상태, P4c).
--
--   run_id       : UUID. 한 방에 여러 run 이력 가능(가장 최근 = updated_at desc).
--   room_key     : 대화 식별자 (= handle_task conv_key = 수신자 bare alias / 그룹 키). 동적.
--   current_step : 0-based 다음(또는 현재 진행 중) 단계 인덱스.
--   status       : running | paused_for_approval | done | failed | cancelled
--   steps_json   : run 시작 시 snapshot 한 단계 배열 + 각 단계 실행 결과(state/result/error)를
--                  누적 기록. [ { label, agent, role, action, state, result?, error? } ... ].
--                  run 시작 후 room_config 가 바뀌어도 이 run 은 snapshot 으로 일관 실행.
--   error        : run-level 실패 사유(있으면). 단계별 사유는 steps_json 안.
--   started_at / updated_at : KST RFC3339 (런타임 주입, 하드코딩 없음).
CREATE TABLE IF NOT EXISTS orchestration_run (
    run_id        TEXT PRIMARY KEY,
    room_key      TEXT NOT NULL,
    current_step  INTEGER NOT NULL DEFAULT 0,
    status        TEXT NOT NULL,
    steps_json    TEXT NOT NULL,
    error         TEXT,
    started_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_orchestration_run_room
    ON orchestration_run(room_key, updated_at DESC);
