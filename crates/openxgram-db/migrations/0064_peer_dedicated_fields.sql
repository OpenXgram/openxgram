-- 0064 — peers 전용 6필드 캐시 컬럼 (list-peer 로스터 단일 진리원천)
-- 마스터 지시(step6): 로스터 6필드(대화명·alias·세션id·역할·폴더위치·활성상태) 중
-- alias/role/session_identifier 는 기존 컬럼으로 충족. 나머지 3개를 전용 컬럼으로 추가한다.
--
-- 원칙(개발원칙 #7 동적 only): 이 컬럼들은 **라이브 실행 상태의 캐시**다. 진리원천은
-- openxgram 라이브 peer + 라이브 tmux/ACP 세션이며, register_subagent(라이브 세션 시작 훅)가
-- 세션 시작 시 이 값을 갱신한다. 비어 있으면 읽기 측이 라이브 상태(tmux cwd, last_seen 등)에서
-- 동적 도출(fallback)한다 — 정적 하드코딩 아님.
ALTER TABLE peers ADD COLUMN display_name TEXT;     -- 대화명(사람이 읽는 표시 이름)
ALTER TABLE peers ADD COLUMN cwd TEXT;              -- 폴더 위치(작업 디렉토리) — 라이브 세션 기준
ALTER TABLE peers ADD COLUMN session_status TEXT;   -- 활성 상태: active | idle | disconnected
CREATE INDEX IF NOT EXISTS idx_peers_display_name ON peers(display_name);
