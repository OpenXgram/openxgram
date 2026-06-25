-- Phase B 통합 현황 그리드 — 토큰단가(token price).
-- 외부 에이전트가 이 에이전트를 사용할 때 1M 토큰당 청구 단가. nullable(미설정=NULL).
-- agent_profiles 확장(중복 보관 안 함). 기존 데이터 무손상(ADD COLUMN).

ALTER TABLE agent_profiles ADD COLUMN token_price_per_million REAL;
