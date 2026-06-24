-- 0066 정본 신원 레코드(peers) 확장 — 지갑 예약 + 충돌해소 arbiter + 내부 route_type
--
-- 전부 nullable, 순수 가산 ALTER. 로직 없음(자리 예약). 정본 신원 레코드는 peers 행
-- (owner key = 기존 public_key_hex / eth_address 재사용 — 중복 컬럼 추가하지 않음).
-- 0065 identity_aliases 는 alias->canonical_address 매핑 테이블일 뿐이라 신원 속성을
-- 담지 않으므로, 속성 컬럼은 peers 에 추가한다.

-- 지갑 (자리 예약, 로직 없음) — IDENTITY-MODEL-FINAL-SPEC §H
-- reserved placeholder, NOT arithmetic source of truth (추후 micro-unit INTEGER 가능)
ALTER TABLE peers ADD COLUMN spending_limit TEXT;   -- nullable, 정수/텍스트 (예: USDC micro)
ALTER TABLE peers ADD COLUMN balance TEXT;          -- nullable
ALTER TABLE peers ADD COLUMN earned TEXT;           -- nullable

-- 이름 유일성 충돌해소 arbiter — IDENTITY-MODEL-FINAL-SPEC §J 갭#1
-- owner public key 는 기존 public_key_hex / eth_address 재사용 (중복 추가 안 함).
ALTER TABLE peers ADD COLUMN origin_machine TEXT;       -- nullable, 등록 원천 머신
ALTER TABLE peers ADD COLUMN identity_version INTEGER;  -- nullable, monotonic 갱신 카운터
-- lease/arbiter 타임스탬프 — 신원 갱신/이어받기 시각(unix ms). version 동률 tie-break 보조.
ALTER TABLE peers ADD COLUMN identity_updated_at INTEGER;  -- nullable, unix ms

-- 내부 route_type (UI 비노출, 운영 디버깅용) — IDENTITY-MODEL-FINAL-SPEC §J 갭#5
ALTER TABLE peers ADD COLUMN route_type TEXT;  -- 예: acp-existing | acp-new | tmux | direct-portal
