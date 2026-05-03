-- Peer registry — 머신 간 통신을 위한 식별·주소록.
-- transport 종류 (HTTP/Tailscale/XMTP) 와 무관하게 peer 메타데이터 통합.
-- ECDSA public_key 는 메시지 서명 검증의 기준.

CREATE TABLE peers (
    id TEXT PRIMARY KEY,
    -- alias 는 사용자가 부여 (예: "mac-mini", "gcp-secondary"). UNIQUE.
    alias TEXT NOT NULL UNIQUE,
    -- ECDSA public key (압축 hex 33 bytes = 66 chars)
    public_key_hex TEXT NOT NULL UNIQUE,
    -- 호출 가능한 주소 — http://ip:port, xmtp://address 등 transport URI scheme
    address TEXT NOT NULL,
    -- primary / secondary / worker
    role TEXT NOT NULL DEFAULT 'worker',
    -- 마지막 연결 시각 (NULL = 미연결)
    last_seen TEXT,
    created_at TEXT NOT NULL,
    notes TEXT
);

CREATE INDEX idx_peers_alias ON peers(alias);
CREATE INDEX idx_peers_public_key ON peers(public_key_hex);
