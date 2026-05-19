-- 0022_users.sql — Web GUI 사용자 인증 (이메일+비밀번호 + JWT).
-- 정본: docs/PRD-OpenXgram.md §4.8 (Web GUI Beta).
--
-- "웹 GUI 진입 시 로그인 → JWT Bearer 로 /v1/gui/* 호출."
--
-- 절대 규칙 1 (fallback 금지): password_hash 미존재 시 401 (silent skip X).
-- 절대 규칙 3 (DB 변경 마스터 승인): 신규 CREATE TABLE 만, 기존 데이터 무영향.
-- mcp_tokens 테이블은 변경 없음 — CLI Bearer 호환 유지.

CREATE TABLE IF NOT EXISTS users (
    id              TEXT PRIMARY KEY,            -- user:<uuid>
    email           TEXT UNIQUE NOT NULL,
    password_hash   TEXT NOT NULL,                -- argon2id encoded ('$argon2id$...')
    alias           TEXT,                          -- 사용자 본인 alias (선택)
    role            TEXT NOT NULL DEFAULT 'user'  -- 'user' | 'admin'
        CHECK (role IN ('user', 'admin')),
    created_at      INTEGER NOT NULL,             -- unix epoch seconds
    updated_at      INTEGER NOT NULL,
    last_login_at   INTEGER
);

CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);

CREATE TABLE IF NOT EXISTS jwt_tokens (
    id              TEXT PRIMARY KEY,            -- uuid (= jti claim)
    user_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash      TEXT NOT NULL,                -- SHA-256(JWT) hex
    issued_at       INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL,
    revoked         INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_jwt_tokens_user ON jwt_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_jwt_tokens_hash ON jwt_tokens(token_hash);
CREATE INDEX IF NOT EXISTS idx_jwt_tokens_expires ON jwt_tokens(expires_at);

-- JWT HS256 서명용 비밀 키. install/init 시 1회 32바이트 OS RNG → hex 저장.
-- daemon 재시작 후에도 동일 키 사용 (기존 JWT 유지).
CREATE TABLE IF NOT EXISTS jwt_secret (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    secret_hex   TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);
