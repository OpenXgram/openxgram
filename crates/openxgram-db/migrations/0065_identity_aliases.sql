-- 정본 신원: 변형 alias -> 정본 주소(머신/에이전트 키스토어 eth_address) 매핑
CREATE TABLE IF NOT EXISTS identity_aliases (
    alias             TEXT PRIMARY KEY,            -- 변형 alias (star, starian, Starian, aoe_star_* 등)
    canonical_address TEXT NOT NULL,               -- 정본 키 = 그룹 대표의 eth_address (없으면 sid:<session>)
    is_primary_alias  INTEGER NOT NULL DEFAULT 0,  -- 1 = 이 신원의 사람 표시용 정본 alias
    status            TEXT NOT NULL DEFAULT 'active', -- active | quarantined
    created_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_identity_aliases_canonical ON identity_aliases(canonical_address);
