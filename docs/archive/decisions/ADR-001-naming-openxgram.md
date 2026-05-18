# ADR-001 — 프로젝트명 OpenXgram 결정

날짜: 2026-04-30
상태: 확정
결정자: 마스터

## 컨텍스트

Akashic의 신체로서 기억·자격 인프라를 구축하는 프로젝트의 이름이 필요했다.

## 결정

프로젝트명: **OpenXgram**

## 근거

- Open: 오픈 프로토콜, 다중 LLM 호환을 강조
- X: XMPP의 X (eXtensible Messaging), XMTP와의 연결, 확장성
- gram: 메시지·기억의 단위 (telegram, anagram의 gram)
- 합쳐서: "확장 가능한 오픈 메모리 메시징 인프라"를 함의

## 트레이드오프

- 긍정: 간결, 기억하기 쉬움, 오픈소스 느낌
- 부정: "gram" 때문에 SNS 서비스로 오해 가능 → README에서 명확히 "기억·자격 인프라"로 정의

## 결과

- 저장소: `openxgram`
- CLI 명령: `xgram`
- 환경변수 접두사: `XGRAM_`
- 설정 디렉토리: `~/.xgram/`
