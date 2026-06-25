-- Phase B 통합 현황 그리드 — 샘플(sample).
-- sample_text  = 샘플 설명/스니펫(텍스트).
-- sample_url   = 샘플 파일 URL 또는 랜딩페이지 URL(한 필드가 둘 다 커버).
-- agent_profiles 확장. nullable. 기존 데이터 무손상(ADD COLUMN).

ALTER TABLE agent_profiles ADD COLUMN sample_text TEXT;
ALTER TABLE agent_profiles ADD COLUMN sample_url TEXT;
