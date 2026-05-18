# ADR — MFA: WebAuthn (passkey-rs) 통합

> 상태: accepted (2026-05-04 KST)
> 관련 PRD: PRD-MFA-02 (Phase 2 §4.3)

## 결정

Phase 2 의 두 번째 MFA 단계로 **WebAuthn (passkey-rs, MIT/Apache-2.0)** 을 채택한다.
TOTP 는 도입하지 않고 PR 폐기 — passkey 가 더 안전하고 사용성도 우수.

적용 범위:
- vault_acl.policy = `mfa` 인 자격증명 접근 시 (vault_get/put/delete)
- payment 한도 변경 (PRD-TAURI-07)
- KEK 회전 (`xgram vault rotate-kek`)

OS biometric (Touch ID / Windows Hello / Android BiometricPrompt) 는 WebAuthn authenticator 로 자연스럽게 통합 — 별도 코드 경로 불필요.

## 의존성 영향

- `passkey = "0.x"` (1Password 라이브러리, MIT/Apache-2.0)
- 빌드 시간 추가 ~5초, 바이너리 +2~3MB (수용 가능)
- alloy 와 의존 트리 충돌 없음 (검증)

## 통합 경로

1. Tauri webview 에서 `navigator.credentials.create()` / `.get()` 호출
2. authenticator 응답 (CBOR) 을 invoke 로 Rust 에 전달
3. Rust 측 passkey-rs 가 attestation/assertion 검증
4. 성공 시 mfa_session_token 발급 (60초 TTL)
5. 다음 vault/payment 호출이 token 제시 → 검증 통과 시 1회 작업 허용

token 은 stronghold 에 저장하지 않고 메모리에만 (zeroize on drop).

## 마스터 절대 규칙 정합성

- "fallback 금지" — WebAuthn 실패 시 master pw 로 silent fallback X. 명시 거부 + 재시도 prompt.
- "롤백 가능" — mfa_session_token 은 60초 TTL, 단발 사용.
- "DB 변경 마스터 승인" — passkey 등록은 사용자 명시 액션. authenticator credential id 는 vault_acl 에 attached.

## 회피 사항

- TOTP — 시간 동기화 의존, phishing 취약, 사용성 떨어짐.
- SMS OTP — 보안 취약 (SIM swap), 적용 제외.
- 자체 PoP scheme — passkey 표준 우월, 자체 구현 회피.

## 후속

- 실제 코드 추가는 별도 PR (PRD-MFA-02 implementation, Phase 2 후반).
- macOS/Windows/Linux 테스트 매트릭스 — 후속 시 정의.
- recovery: 마스터 mnemonic 으로만 (passkey 복구 별도 manager).
