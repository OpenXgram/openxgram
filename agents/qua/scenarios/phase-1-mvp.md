# Qua Phase 1 MVP 검증 시나리오집

작성: 2026-05-03 KST · 대상: OpenXgram v0.1.0.0-alpha.1 Phase 1 MVP

## 0. 문서 위치·범위

이 문서는 D(Keystore)·E(DB) 산출물을 즉시 검증하기 위한 실행 가능 시나리오집이다.
SPEC-memory-transfer §13, SPEC-lifecycle §13, PRD §20을 베이스로, Phase 1 MVP 범위에 한정해 분해했다.

검증 보고 형식 (Qua CLAUDE.md 준수):
- 코드 검증: 문법 에러 없음 (`cargo check`, `cargo clippy -- -D warnings`)
- 실제 작동: 직접 실행한 결과만 "작동"으로 인정
- 보고: `코드 검증: ✅ / 실제 작동: ✅` 또는 `❌`

ID 체계:
- KS-Uxx — Keystore 단위 (D 산출물)
- DB-Uxx — Database/Migration 단위 (E 산출물)
- SE-xx — Silent Error 4패턴 (절대 규칙 14.1)
- MT-Ixx — Memory Transfer 통합
- LC-Ixx — Lifecycle 통합
- VA-xx — Vault ACL
- TR-xx — Transport 라우팅
- EM-xx — 임베딩·회상
- SM-xx — 세션 이동성
- AD-xx — Discord/Telegram 어댑터
- MV-xx — 마스터 검증 시나리오 (Phase 1 합격 기준)

각 시나리오 형식: ID · 목적 · 사전 조건 · 실행 · 합격 기준 · silent error 게이트.

---

## 1. Keystore 단위 — D 산출물 검증

### KS-U01 secp256k1 키페어 생성

- 목적: 새 키페어 생성 시 공개키·EIP-55 주소 일관성 확인
- 사전: D 머지 완료, `cargo test -p openxgram-keystore` 가능 상태
- 실행: `Keystore::generate()` → 공개키 65바이트(uncompressed) → keccak256 → 마지막 20바이트 → EIP-55 체크섬
- 합격: 같은 시드에서 동일 주소 재현. EIP-55 체크섬 케이스 정확. `0x` prefix.
- silent error 게이트: `unwrap()` 0건, 모든 오류 `?` propagation.

### KS-U02 BIP39 24단어 시드 생성·검증

- 목적: BIP39 엔트로피·체크섬 정확성
- 사전: 동일
- 실행: 256-bit 엔트로피 → 24단어 mnemonic → seed (PBKDF2-HMAC-SHA512, 2048 iter) → 64바이트 seed
- 합격: BIP39 영문 워드리스트 정확 매칭. 체크섬 비트 일치. 표준 테스트 벡터 통과(예: `abandon abandon... art`).
- silent error 게이트: 잘못된 워드리스트 검증 시 raise — 조용히 false 반환 금지.

### KS-U03 BIP44 HD 파생 결정성

- 목적: `m/44'/60'/0'/0/N` 경로 파생 결정성
- 사전: 동일
- 실행: 같은 seed에서 N=0,1,2,...,5 파생 → 각각 결정된 주소 생성
- 합격: 동일 seed → 동일 N → 항상 같은 주소. ethers.js·MetaMask와 cross-validation.
- silent error 게이트: hardened 비트(`'`) 누락 시 raise.

### KS-U04 서브에이전트 키 자동 파생

- 목적: `m/44'/60'/parent'/0/task_seq` 패턴 파생
- 사전: 동일
- 실행: parent=0 영구 에이전트 → task_seq=1,2,3 서브에이전트 키 발급
- 합격: 각 task_seq별 다른 주소. 같은 task_seq → 같은 주소(결정성).
- silent error 게이트: 누락된 component → 0 fallback 금지, raise.

### KS-U05 Keystore V3 ChaCha20-Poly1305 round-trip

- 목적: 디스크 저장 후 복호화 일치
- 사전: tempdir
- 실행: privkey + 패스워드 → V3 JSON 저장 → 같은 패스워드로 read → 같은 privkey 복원
- 합격: hex 일치. cipher=`chacha20-poly1305`, kdf=`argon2id`. 잘못된 패스워드 → MAC mismatch raise.
- silent error 게이트: MAC 검증 실패 시 raise(절대 빈 키 반환 금지).

### KS-U06 Argon2 KDF 결정성

- 목적: 동일 패스워드+salt → 동일 키
- 사전: 동일
- 실행: argon2id, m=64MB, t=3, p=4 (또는 합의된 파라미터) → 32바이트 derived key
- 합격: 결정성. salt 다르면 다른 키. 파라미터 메타가 V3 JSON에 정확 기록.
- silent error 게이트: argon2 실패 시 raise.

### KS-U07 zeroize Drop 검증

- 목적: privkey/seed Drop 시 메모리 제로화
- 사전: `zeroize::ZeroizeOnDrop` 적용
- 실행: 키 객체를 unsafe로 메모리 주소 추적 → drop → 같은 주소 읽기
- 합격: 모든 바이트 0. 또는 valgrind/miri로 사용 후 무효화 검증.
- silent error 게이트: derive 매크로 누락 시 컴파일 실패하도록 단위 테스트가 강제.

### KS-U08 BIP39 import 일치

- 목적: 외부 시드(예: MetaMask) import → 같은 주소
- 사전: 알려진 테스트 시드
- 실행: 24단어 import → m/44'/60'/0'/0/0 → 주소 비교
- 합격: 외부 도구(ethers.js)와 동일 주소.
- silent error 게이트: 길이 != 24 → raise. checksum mismatch → raise.

### KS-Cli01 xgram keypair 명령

- 목적: CLI 노출 검증 (D PR에 포함된 명령)
- 실행: `xgram keypair generate`, `xgram keypair show`, `xgram keypair import --mnemonic "..."`
- 합격: 표준 출력 JSON 또는 표준 형식. 종료 코드 0=성공, !=0 raise. 패스워드 stdin 입력 정상.

---

## 2. Database/Migration 단위 — E 산출물 검증 (이미 main 머지: ab4186c)

### DB-U01 Db::open 초기화

- 목적: 새 DB 파일 생성 + sqlite-vec extension 로드
- 사전: tempdir, 빈 경로
- 실행: `Db::open(path)` → 파일 생성 + `vec0` 가상 테이블 사용 가능
- 합격: 파일 존재. `SELECT vec_version()` 호출 성공. PRAGMA `journal_mode=WAL`, `foreign_keys=ON`.
- silent error 게이트: extension 로드 실패 시 raise — 빈 fallback 금지.

### DB-U02 MigrationRunner 0001_init 적용

- 목적: 첫 마이그레이션 적용 후 schema_version=1
- 사전: 빈 DB
- 실행: `MigrationRunner::run(&db)` → `schema_migrations` 테이블 + 0001_init 실행
- 합격: `SELECT version FROM schema_migrations` → 1. 5개 테이블(sessions/messages/memories/contacts/share_policy) 존재.
- silent error 게이트: 마이그레이션 SQL 실패 시 raise + rollback.

### DB-U03 Idempotent migrate

- 목적: migrate 2회 연속 안전
- 사전: 1회 적용 완료 상태
- 실행: `MigrationRunner::run(&db)` 재호출
- 합격: 에러 없음. schema_version 변동 없음. 테이블 중복 생성 없음.
- silent error 게이트: 이미 적용된 마이그레이션 skip을 명시 로그(`tracing::info`).

### DB-U04 5개 테이블 스키마 검증

- 목적: 0001_init.sql의 테이블·컬럼·제약 정확성
- 실행: `PRAGMA table_info(<table>)` 각 테이블
- 합격: SPEC §9 (MT) 데이터 모델과 컬럼명·타입·NOT NULL·PRIMARY KEY 일치.
- 추가: foreign key 관계(messages→sessions, memories→messages) 검증.

### DB-U05 vec0 가상 테이블

- 목적: sqlite-vec embedding 컬럼 사용 가능
- 실행: `CREATE VIRTUAL TABLE temp.test_vec USING vec0(embedding float[384])` (e5-small 384차원)
- 합격: 정상 생성. INSERT/SELECT 동작.
- silent error 게이트: dim mismatch → raise.

### DB-U06 affected_rows 검증

- 목적: rusqlite UPDATE/DELETE 0건 → raise (silent error 4패턴 #2)
- 실행: 존재하지 않는 ID로 UPDATE → `affected_rows() == 0` → 명시적 raise
- 합격: `Err(StoreError::NotFound)` 반환. log entry 1건.
- silent error 게이트: 이 시나리오 자체가 silent error 게이트.

### DB-U07 동시 트랜잭션 (WAL)

- 목적: WAL 모드에서 동시 read+write 정상
- 실행: tokio task 2개 — 한쪽 INSERT 트랜잭션, 다른 쪽 SELECT
- 합격: 데드락 없음. SELECT는 commit 전 데이터 미관측, commit 후 관측.

### DB-U08 외래키 제약 raise

- 목적: 부모 없는 자식 INSERT → 즉시 raise
- 실행: 존재하지 않는 session_id로 messages INSERT
- 합격: `SQLITE_CONSTRAINT_FOREIGNKEY` 에러로 raise (PRAGMA foreign_keys=ON 동작 확인).

---

## 3. Silent Error 4패턴 — 절대 규칙 14.1 (모든 PR 게이트)

PRD §14.1 + Phase 1 MVP 체크리스트 "코드 리뷰 체크리스트" 항목.

### SE-01 reqwest .error_for_status()? 강제

- 목적: 4xx/5xx 응답이 `Ok`로 흡수되지 않음
- 실행: 모의 서버 401/500 응답 → 사이드카 outbound 호출
- 합격: `Result::Err` 반환. 에러 메시지에 status code 포함. tracing::error 로그 1건.
- 코드 리뷰 게이트: `grep -rn "reqwest" crates/ | grep -v "error_for_status"` 결과 0건 (allow-list 외).

### SE-02 rusqlite affected_rows 검증

- 목적: UPDATE/DELETE 0건 → raise
- 실행: DB-U06과 동일
- 합격: 모든 UPDATE/DELETE 호출이 `affected_rows()` 검사 포함. 0건이면 raise.
- 코드 리뷰 게이트: `grep -rn "execute(" crates/ | grep -v "affected"` 검토.

### SE-03 tokio-cron-scheduler panic 핸들러

- 목적: job 내부 panic이 silent 흡수되지 않음
- 실행: 의도적 panic을 발생시키는 job → tracing 로그 + raise 검증
- 합격: panic 발생 시 `tracing::error!(panic = %p, "job panicked")` 1건. job 등록 시 catch_unwind 또는 panic_hook 적용.
- 코드 리뷰 게이트: 모든 job 등록부에 panic 핸들러 존재.

### SE-04 keyring round-trip get

- 목적: headless Linux silent 실패 방지
- 실행: 더미 secret 저장 → 즉시 `get` → 일치 검증
- 합격: 일치. 불일치/None 시 raise. headless Linux Docker(`apt remove gnome-keyring`) 환경에서 raise 동작 확인.
- silent error 게이트: 이 round-trip 검증 누락 시 코드 리뷰 reject.

---

## 4. Memory Transfer 통합 — SPEC-MT §12 분해

### MT-I01 ChatGPT 웹 토론 → import (PRD §20 F, 마스터 핵심 요구)

- 목적: 외부 LLM 토론 결과를 사이드카로 흡수
- 사전: D·E 머지, MT extract/import 구현
- 실행: ChatGPT 토론 텍스트 → `xgram extract --format text-package --target clipboard` → 외부 창 붙여넣기 → 응답 복사 → `xgram session import --from clipboard`
- 합격: L0 messages 신규 row, L1 episode 1건, 임베딩 384차원 생성. session_id 일관성. audit_log entry 1건.
- silent error 게이트: 파싱 실패 시 `~/.openxgram/failed/`에 보존 + raise.

### MT-I02 Discord webhook outbound

- 목적: 사이드카 → Discord webhook 발송 + 서명
- 실행: `xgram backup-push --target discord --channel <id>`
- 합격: HMAC-SHA256 서명 헤더 포함. 4000자 초과 시 자동 분할(파일 attach). transfer_logs status=`sent`.
- silent error 게이트: 401/403/429 → retry 3회(30s/60s/120s) → 모두 실패 시 raise + Telegram 알림.

### MT-I03 Inbound webhook 서명 검증

- 목적: 외부 시스템(Linear) → 사이드카 inbound + 서명 거부
- 실행: 위조 서명으로 POST → 401. 정상 서명으로 POST → 200 + L2 memory 1건.
- 합격: 위조 서명 → 401 + transfer_logs `status=rejected, error_message=SIGNATURE_INVALID`. 5회 연속 실패 → 1시간 차단 + 마스터 Telegram 알림.
- silent error 게이트: 서명 검증 실패 silent allow 절대 금지.

### MT-I04 on-schedule 18시 자동 백업

- 목적: tokio-cron-scheduler 정시 트리거
- 실행: KST 18:00 cron 등록 → 시간 mock 또는 즉시 트리거 → Discord 채널 push
- 합격: 매일 18:00 KST에 정확히 1회 트리거. job panic 시 SE-03 동작.

### MT-I05 다른 머신 sync (Tailscale + 시드 서명)

- 목적: 머신 A → 머신 B 메모리 동기화
- 실행: 머신 A `xgram session export` → Tailscale 전송 → 머신 B `xgram session import`
- 합격: mTLS 핸드셰이크 성공. 시드 서명 검증 통과. L0~L4 모두 동일.

---

## 5. Lifecycle 통합 — SPEC-Lifecycle §13 분해

### LC-I01 install → uninstall 흔적 0건 (PRD §20 H)

- 목적: 라운드트립 후 시스템 흔적 없음
- 실행: `xgram init`(비대화) → `xgram uninstall --full-backup` → 검사
- 합격: `~/.openxgram/` 없음. systemd unit 없음. shell rc 마커 0건. keyring entry 0건. 흔적 검사기 0건.
- silent error 게이트: 사후 검증(SPEC §5.5) 실패 시 raise.

### LC-I02 install → reset --hard --keep-keys → 즉시 재사용 (PRD §20 I)

- 목적: 데이터 초기화 후 키 보존
- 실행: init → 사용 → `reset --hard --keep-keys` → `doctor`
- 합격: doctor 모든 항목 OK. 같은 secp256k1 주소. DB 빈 상태.

### LC-I03 10회 install-uninstall (PRD §20 J)

- 목적: 마스터 반복 워크플로우 안정성
- 실행: 자동화 스크립트로 10회 round-trip
- 합격: 매회 흔적 0건. doctor OK. 누적 디스크 누수 없음.

### LC-I04 doctor 헬스체크 10+

- 목적: SPEC-Lifecycle §6 핵심 항목 1~7 동작
- 실행: `xgram doctor`
- 합격: 데몬 PID·uptime, DB 무결성, keystore 잠금, Tailscale 연결, Discord/Telegram 토큰 유효성, 디스크 사용량, 포트 바인딩 모두 검사 + OK/WARN/FAIL 출력.

### LC-I05 drift detection

- 목적: 외부에서 install-manifest 수정 감지
- 실행: 사용자가 `~/.openxgram/install-manifest.json` 수동 변경 → `xgram doctor`
- 합격: drift 항목 FAIL + 차이 출력. 자동 복구는 Phase 1.5 이후.

### LC-I06 Idempotent uninstall 2회

- 목적: 두 번 uninstall 안전
- 실행: uninstall → uninstall 재실행
- 합격: 두 번째 호출이 NoOp 또는 graceful "이미 제거됨" 메시지. 종료 코드 0.

---

## 6. Vault ACL 침투 (Phase 1 기본)

### VA-01 권한 없는 에이전트 거부

- 목적: ACL 외 에이전트의 vault 접근 거부
- 실행: 미등록 secp256k1 주소로 vault read 시도
- 합격: 403/`PermissionDenied`. audit_log entry 1건.

### VA-02 일일 한도 초과

- 목적: rate limit 동작
- 실행: 일일 한도(예: 100회) 초과 호출
- 합격: 한도 초과 후 `RateLimitExceeded` raise. UTC가 아닌 KST 자정 기준 reset.

### VA-03 머신 화이트리스트 외 거부

- 목적: 등록 외 머신 IP 거부
- 실행: 화이트리스트에 없는 Tailscale IP로 호출
- 합격: 거부 + audit_log.

### VA-04 mfa 정책

- 목적: mfa 자격증명은 MFA 없이 접근 거부
- 실행: mfa 정책 자격에 confirm 없이 접근
- 합격: 거부 + 마스터 Telegram MFA 챌린지 발송.

---

## 7. Transport 라우팅 (PRD §4)

### TR-01 localhost 우선

- 사전: 같은 머신 IPC 가능
- 실행: send → IPC 경로 사용
- 합격: tracing 로그 `transport=ipc`. Tailscale·XMTP 호출 0건.

### TR-02 Tailscale 단계

- 사전: localhost 불가, Tailscale 가능
- 실행: send → Tailscale 경로
- 합격: 로그 `transport=tailscale`. mTLS 핸드셰이크 성공.

### TR-03 XMTP 단계

- 사전: 양쪽 Tailscale 불가
- 실행: send → XMTP REST (reqwest 직접)
- 합격: 로그 `transport=xmtp`. `.error_for_status()?` 동작.

### TR-04 명시적 단계 — silent fallback 금지

- 목적: 자동 전환 시 모든 단계 변경을 로그
- 합격: 모든 transport 변경에 `tracing::info!(from, to)` 로그 1건. 사용자 모르게 다운그레이드 금지.

---

## 8. 임베딩·회상

### EM-01 multilingual-e5-small 한국어

- 목적: 한국어 임베딩 정상
- 실행: 한국어 문장 → 384차원 vec
- 합격: f32 384개. NaN/Inf 없음. cosine 유사도가 한국어 의미와 일치(예: "안녕"≈"hi" 높음, "안녕"≈"피자" 낮음).

### EM-02 회상 복합 점수

- 목적: α·β·γ·δ 가중 정확성
- 실행: 의미·시간·핀·접근빈도 4축 점수 계산 → 정렬
- 합격: PRD §5에 따른 가중치 적용. tied score 결정성.

### EM-03 임베딩 실패 raise

- 목적: 모델 로드 실패 시 raise (--embed defer 외)
- 실행: 모델 파일 삭제 후 호출
- 합격: raise + 마스터 알림. defer 모드 외에는 silent skip 금지.

### EM-04 embedding_queue 재시도

- 목적: defer 모드 큐 재시도
- 실행: defer 모드에서 임베딩 실패 → 큐 등록 → 야간 reflection 재시도
- 합격: 3회 실패 시 raise + 로그.

---

## 9. 세션 이동성 (PRD §20 C)

### SM-01 GCP → Mac Mini 이동

- 목적: 세션 export → 다른 머신 import
- 실행: GCP `xgram session export <id>` → Mac Mini `xgram session import`
- 합격: 같은 session_id 재사용. L0~L4 모두 동일.

### SM-02 신원 연속성

- 목적: 같은 secp256k1 주소
- 합격: keystore에 같은 마스터 시드 import → 동일 주소.

### SM-03 메모리 보존

- 합격: 메시지 개수, episode summary, traits 모두 일치.

---

## 10. Discord/Telegram 어댑터

### AD-01 Discord 모델 C (봇 1개 + webhook 발신자 분리)

- 합격: 같은 봇이 Webhook URL로 발신자명을 변경해 메시지 발송. 토큰 유효성 doctor에서 검증.

### AD-02 Telegram Setup Agent 1:1

- 합격: 마스터 chat_id 1:1 메시지 발송. 4096자 초과 자동 분할.

### AD-03 토큰 만료 감지

- 합격: 401 응답 시 doctor에서 FAIL. 마스터 알림.

---

## 11. 마스터 검증 시나리오 (Phase 1 합격 기준 5개)

PRD §20에서 Phase 1 MVP 적용:

- MV-A — 에이전트 간 기억 공유 + 검증 요청 (PRD §20 A)
- MV-B — Vault 키 자동 공유 (PRD §20 B)
- MV-D — NEW / ROUTINE 자동 분류 (PRD §20 D)
- MV-E — 파일 송수신 (PRD §20 E)
- MV-F — ChatGPT 웹 토론 → 사이드카 import → Claude Code attach (PRD §20 F, 마스터 핵심 요구)

각 시나리오는 PRD §20에 단계 정의됨. Qua는 단계 그대로 실행하고 합격 기준 충족 여부를 보고한다.

이 5개 모두 통과해야 Phase 1 MVP 합격 — 마스터 확정 기준.

---

## 12. 실행 순서 권고

D 머지 직후:
- 1단계: KS-U01 ~ KS-U08 + KS-Cli01 (Keystore 단위)
- 2단계: SE-01 ~ SE-04 (silent error — 모든 PR 게이트)

E 이미 머지(ab4186c) — 즉시 가능:
- DB-U01 ~ DB-U08

D·E 모두 완료 후:
- LC-I01 ~ LC-I06 (Lifecycle init/uninstall 구현 후)
- MT-I01 ~ MT-I05 (MT 모듈 구현 후)
- MV-A ~ MV-F (Phase 1 합격 검증, 마지막)

VA·TR·EM·SM·AD는 해당 모듈 구현 후 단계적 적용.

---

## 13. 도구·환경

- Rust: `cargo test --workspace --all-features`
- 통합: `cargo test --test integration_*`
- mock 서버: `wiremock` (reqwest mocking)
- DB tempdir: `tempfile::tempdir()`
- headless Linux 검증: Docker `ubuntu:22.04` minimal (gnome-keyring 미설치)
- 시간 mock: `tokio::time::pause` + `advance`
- coverage 목표: 단위 80%, 통합 핵심 경로 100%

---

## 14. 보고 템플릿

```
[Qua 검증 보고 — Phase 1 MVP {YYYY-MM-DD KST}]
실행 시나리오: KS-U01 ~ KS-U08 (8건)
- 코드 검증: ✅ (cargo check 통과, clippy 0 warnings)
- 실제 작동: ✅ (8/8 통과, 실행 시간 0.5s)
silent error 게이트: ✅ (reqwest/rusqlite/cron/keyring 4패턴 모두 적용 확인)
실패: 없음 / 또는 KS-U05 MAC 검증 라인 누락 — Eno 수정 요청
```

---

## 15. 변경 이력

- 2026-05-03 KST: 초안 작성. D·E 산출물 즉시 검증 가능 시나리오집 + Phase 1 MVP 합격 기준 5개 명시.
