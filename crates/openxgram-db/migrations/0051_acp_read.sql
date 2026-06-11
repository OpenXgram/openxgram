-- ACP 대화 읽음 상태 — 메신저식 안읽음 배지/정렬용. conv_key(에이전트 alias)별 마지막 읽은 시각.
CREATE TABLE IF NOT EXISTS acp_read (
    conv_key   TEXT PRIMARY KEY,
    last_read  TEXT NOT NULL
);
