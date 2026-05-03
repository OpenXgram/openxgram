# Security Policy / 보안 정책

## 지원 버전 / Supported Versions

현재 MVP 개발 단계입니다. 아래 버전이 보안 패치를 받습니다.

| 버전 | 지원 여부 |
|------|-----------|
| v0.x (alpha) | 지원 (모든 v0.x 버전) |

v1.0 정식 릴리즈 이후에는 최신 마이너 버전만 지원합니다.

---

## 취약점 신고 / Reporting a Vulnerability

**GitHub Issues에 보안 취약점을 공개하지 마세요.**

GitHub Security Advisories를 사용해 비공개로 신고해 주세요:

1. 이 저장소의 **Security** 탭으로 이동
2. **"Report a vulnerability"** 클릭
3. 상세 내용 작성 후 제출

또는 이메일: `security@openxgram.org` (placeholder — 도메인 미확정)

---

## 신고 내용에 포함할 사항

- 취약점 유형 (예: RCE, 권한 상승, 키 노출)
- 영향받는 컴포넌트 및 버전
- 재현 단계 (PoC 코드 또는 스크린샷 포함 시 도움됨)
- 예상되는 영향 범위

---

## 응답 시간 / Response Timeline

| 단계 | 목표 시간 |
|------|-----------|
| 첫 응답 | 48시간 이내 |
| 초기 평가 완료 | 7일 이내 |
| 패치 릴리즈 | 심각도에 따라 7~30일 |

---

## 책임 공개 정책 / Responsible Disclosure Policy

OpenXgram은 책임 있는 공개 원칙을 따릅니다:

- 신고 접수 후 **90일** 이내에 패치를 제공하는 것을 목표로 합니다
- 90일 이내 해결이 어려운 경우 신고자와 협의하여 공개 일정을 조율합니다
- 패치 릴리즈 후 신고자를 Security Advisory에 공개 크레딧으로 인정합니다 (원하는 경우)
- 신고자가 법적 문제 없이 조사할 수 있도록 선의의 연구를 지원합니다

---

## 범위 / Scope

다음은 보안 신고 범위에 포함됩니다:

- `crates/openxgram-keystore` — 키페어 생성, 저장, 파생 관련 취약점
- `crates/openxgram-db` — SQLite 데이터 노출, 암호화 관련 취약점
- `crates/openxgram-daemon` (예정) — 데몬 프로세스 권한 상승
- `xgram` CLI — 임의 코드 실행, 경로 순회

다음은 범위 외입니다:

- 의존성 라이브러리 자체의 취약점 (해당 라이브러리에 직접 신고)
- 소셜 엔지니어링 공격
