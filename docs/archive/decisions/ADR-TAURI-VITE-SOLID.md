# ADR — Tauri R/W: Vite + Solid.js + TypeScript 마이그레이션

> 상태: accepted (2026-05-04 KST)
> 관련 PRD: PRD-TAURI-01 ~ 09 (Phase 2.3)

## 결정

정적 HTML 폐기하고 ui/tauri/app/ 디렉토리에 **Vite + Solid.js + TypeScript** 베이스로 R/W 데스크톱 앱을 마이그레이션.

근거:
- React 보다 50%+ 작은 번들 (~10KB), signal 기반 = Tauri Channel 스트림과 천연 적합
- 가상 스크롤(@tanstack/solid-virtual), 폼 검증(zod), i18n(@solid-primitives/i18n) 생태계 충분
- 기존 정적 ui/tauri/frontend 는 ui/tauri/legacy/ 보관 (참고용)

## 7 액션 (PRD §3.5)

5개 우선순위 탭 + 2개 후속 (peer 추가, payment 한도) 의 component 가 ui/tauri/app/src/components/ 에 위치:

1. PendingList.tsx — vault 자격증명 승인/거부 (PRD-TAURI-03)
2. SearchView.tsx — L0~L4 디바운스 검색 (PRD-TAURI-04)
3. PeersView.tsx — peer 추가 + fingerprint 확인 (PRD-TAURI-05)
4. VaultRevealView.tsx — 30초 마스킹 + clipboard 자동 클리어 (PRD-TAURI-06)
5. PaymentLimitsView.tsx — 한도 변경 + MFA 재인증 (PRD-TAURI-07)
6. (deferred) Pin/unpin memory 우클릭 메뉴
7. (deferred) Episode 강제 종료/시작

## Tauri Plugins (PRD §3.2)

- stronghold — master pw 캐싱·세션 키
- dialog — confirm/ask
- clipboard-manager — vault_get 30초 자동 클리어
- notification — 새 pending 알림
- updater — GitHub Releases + minisign
- store — 사용자 locale 등 UI 설정
- global-shortcut — Ctrl+Shift+O quick-open
- single-instance — 중복 실행 방지

package.json 에 모두 dependency 등록. capabilities/main.json 에 명시 권한 (와일드카드 금지).

## 보안 정책 (PRD §3.4)

- `withGlobalTauri: false` — JS 네임스페이스 오염 방지
- CSP `default-src 'self'; connect-src ipc: http://ipc.localhost`
- vault 평문 invoke 응답 직접 X — Stronghold ephemeral token 우선 (현 데모는 plaintext, 후속 강화)
- capabilities 와일드카드 금지

## i18n (PRD-TAURI-09)

- @solid-primitives/i18n
- ko.json + en.json 카탈로그 (ui/tauri/app/src/i18n/)
- 첫 진입 시 navigator.language 로 자동 감지, localStorage("locale") 로 사용자 override 보존

## 자동 업데이트 (PRD-TAURI-08)

- @tauri-apps/plugin-updater
- GitHub Releases 의 latest.json + minisign 서명
- 서명 키 관리는 별도 docs/release-signing.md (TODO 후속)

## 빌드

```bash
cd ui/tauri/app && npm install && npm run build       # → app/dist
cd ui/tauri && cargo tauri dev                          # frontend dev server 자동 시작
cd ui/tauri && cargo tauri build                        # 단일 바이너리 + installer
```

빌드 결과 < 200KB JS bundle 목표.

## 후속

- 실 backend 통합 — invoke command 매핑 (vault_pending_list, memory_search, peers_list, vault_get, payment_get_daily_limit) 은 ui/tauri/src/lib.rs 에서 지원 (Phase 2.3 후속 PR).
- minisign 서명 키 관리 docs (PRD-TAURI-08 후속).
- E2E 테스트 (Playwright + tauri webdriver) — Phase 3.
