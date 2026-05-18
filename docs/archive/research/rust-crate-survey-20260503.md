# OpenXgram Phase 1 MVP — Rust 크레이트 조사 보고서

작성일: 2026-05-03
작성자: Res (리서치 에이전트)
목적: Phase 1 MVP 의존성 선정을 위한 크레이트 성숙도·라이선스·함정 점검

---

## 1. 카테고리별 권고

---

### 1. 키페어·시드 (블록체인 강도)

**권고: `k256` + `bip39` + `coins-bip32` + `alloy-signer`**

`k256`은 RustCrypto 프로젝트의 secp256k1 순수 Rust 구현이다.
- 안정 버전: 0.13.4 (2024-09-20). 현재 0.14.0-rc.9 진행 중으로 stable을 사용해야 한다.
- 라이선스: Apache-2.0 OR MIT
- 다운로드: 5300만+, RustCrypto 생태계 내 핵심 크레이트
- 주의: NCC Group 감사에서 ECDSA/Schnorr 고심각도 이슈가 발견된 바 있으나 이미 수정됨. 0.13.x stable 사용 권고.

`bip39` v2.2.2 (2025-12-04)는 24단어 니모닉 생성/복원을 담당한다.
- 라이선스: CC0-1.0 (퍼블릭 도메인 수준, 상업적 사용 완전 자유)
- 다운로드: 1056만+

`coins-bip32` v0.13.0 (2025-07-29)은 BIP32 HD 파생 경로(m/44'/60'/N'/0/M)를 지원한다.
- 라이선스: MIT OR Apache-2.0
- `bip32` 크레이트(docs.rs/bip32)도 대안이지만 coins-bip32가 alloy 생태계와 통합성이 높다.

`alloy-signer` v2.0.4 (2026-04-29)는 EVM 체인 트랜잭션 서명에 사용한다.
- 라이선스: MIT OR Apache-2.0
- Base 체인(L2, EVM 호환) 직접 지원. ethers 대비 활발한 유지보수.
- ethers v2.0.14는 2024-03-06 이후 업데이트 없음 — 사실상 deprecated. alloy로 마이그레이션 완료 상태.

대안: `secp256k1` v0.31.1 (Bitcoin Core C 라이브러리 FFI 바인딩). CC0-1.0 라이선스. 성능은 높지만 C 의존성으로 크로스 컴파일 복잡도 증가. k256 순수 Rust보다 불필요.

---

### 2. 데이터베이스

**권고: `rusqlite` + `sqlite-vec`**

`rusqlite` v0.39.0 (2026-03-15)은 SQLite 동기 드라이버의 표준이다.
- 라이선스: MIT
- 다운로드: 5900만+
- SQLite 확장 로딩 지원. loadable_extension feature로 sqlite-vec 연동 가능.
- 트랜잭션 및 파라미터 바인딩 완성도 높음.

`sqlite-vec` v0.1.9 (2026-03-31)은 Rust 바인딩이 공식 존재한다(crates.io 검증 완료).
- 라이선스: MIT OR Apache-2.0
- Alex Garcia(asg017)가 sqlite-vec C 소스를 cc 크레이트로 빌드타임에 정적 링크하는 방식.
- `sqlite3_vec_init` 함수 하나를 노출하며, rusqlite의 load_extension과 연동.
- 주의: v0.1.x alpha 단계. float/int8/binary 벡터 vec0 가상 테이블 지원.

`sqlx` 대안 평가: v0.8.6 stable (2025-05-19). MIT OR Apache-2.0. async/await 지원으로 tokio와 자연스럽게 통합. 단, 0.9.0-alpha.1까지만 있어 안정판이 v0.8.6이며 sqlite-vec 확장 로딩 지원이 rusqlite보다 번거롭다. sqlite-vec와 함께 쓰려면 rusqlite가 직접적이다. 만약 async 쿼리가 핵심이라면 sqlx 채택을 고려할 수 있으나 두 드라이버를 같이 쓰는 것은 의존성 낭비다.

결정 필요: rusqlite(sync) vs sqlx(async) 중 하나를 선택해야 한다. Phase 1에서 async DB가 필수적이지 않다면 rusqlite로 단순화를 권고한다.

---

### 3. 임베딩

**권고: `fastembed` (BGE-small ONNX 로컬)**

`fastembed` v5.13.4 (2026-04-27)는 ONNX 모델 기반 로컬 임베딩 생성 크레이트다.
- 라이선스: Apache-2.0
- 다운로드: 96만+, Qdrant 팀이 관리. 활발한 유지보수.
- BGE-small-EN-v1.5, multilingual-e5-small 등 사전 정의 모델을 자동 다운로드·캐시.
- 내부적으로 ort를 사용하므로 ONNX Runtime 설치 필요.
- 한국어 처리: multilingual-e5-small 모델 선택 시 한국어 멀티바이트 지원 가능. BGE-small은 영어 전용이므로 한국어가 중요하면 모델 선택에 주의 필요.

`ort` v1.16.3 stable (2025-11-20). v2.0.0-rc.12 진행 중.
- 라이선스: MIT OR Apache-2.0
- fastembed가 ort에 의존하므로 직접 사용 필요는 낮지만, ONNX 모델을 직접 로드할 경우 사용.
- 주의: v2.0은 API가 대규모 변경됨 (SessionBuilder → Session::builder, ndarray 0.17 업그레이드). fastembed 5.x가 어느 버전을 사용하는지 Cargo.lock으로 확인 권고.

`candle-core` v0.10.2 (2026-04-01)은 HuggingFace의 순수 Rust ML 프레임워크다.
- 라이선스: MIT OR Apache-2.0
- ONNX Runtime C 라이브러리 없이 동작하나 모델 최적화 수준이 낮고 임베딩 전용 high-level API 부재. Phase 1에서 fastembed 대비 설정 복잡도가 높다. Phase 2+ 검토 대상.

---

### 4. 메시징·네트워크

**권고: `tokio` + `reqwest` + `axum`**

`tokio` v1.52.1 (2026-04-16). MIT. 다운로드 6.4억+. Rust async 생태계의 표준 런타임. 이견 없음.

`reqwest` v0.13.3 (2026-04-27). MIT OR Apache-2.0. 다운로드 4.6억+. HTTP 클라이언트 표준. tokio 기반 async 지원. MCP·REST API 호출에 사용.

`axum` v0.8.9 (2026-04-14). MIT. 다운로드 3억+. tokio-team이 관리하는 HTTP 서버 프레임워크. inbound webhook 처리에 적합.

`tonic` v0.14.5 (2026-02-19). MIT OR Apache-2.0. gRPC가 필요한 경우만 도입. Phase 1에서는 불필요.

`libp2p` v0.56.0 (2025-06-27)은 P2P 보조 옵션으로 언급되었으나 의존성이 매우 무겁다. Phase 1 MVP에서는 제외 권고.

**XMTP Rust SDK 현황 (중요 우려 사항)**
crates.io에 `xmtp` v0.9.3이 존재하나 이는 qntx라는 개인 개발자가 만든 비공식 FFI 래퍼(다운로드 731회)다. 공식 XMTP 조직(xmtp-org)의 `libxmtp`는 Rust 코어로 작성되어 있지만 crates.io에 게시되지 않았다. Node/Swift/Kotlin 바인딩을 우선 지원하며, Rust 공개 크레이트는 존재하지 않는다. WASM과 Android가 best-supported 상태라는 공식 언급이 있다. 결론: Phase 1에서 XMTP를 사용하려면 reqwest로 REST API를 직접 호출하거나, libxmtp 소스를 로컬 의존성으로 포함하는 방식이 유일한 선택지다. 공식 Rust 크레이트가 없으므로 XMTP 통합은 어댑터 패턴으로 reqwest 기반 HTTP 래퍼를 직접 구현해야 한다.

---

### 5. 어댑터

**Discord: `serenity` v0.12.5 (2025-12-20)**
- 라이선스: ISC (MIT 호환, 상업적 사용 가능)
- 다운로드: 510만+
- tokio async, 봇 이벤트 핸들러, 슬래시 커맨드 지원 완성도 높음.
- 대안 `twilight-model` v0.17.1 (2025-12-13)은 모듈식이나 조립 복잡도 높음. Phase 1은 serenity가 적합.

**Telegram: `teloxide` v0.17.0 (2025-07-11)**
- 라이선스: MIT
- 다운로드: 111만+
- tokio 기반 FSM 대화 흐름, 커맨드 파서 내장.
- 대안 `frankenstein`은 경량이지만 고수준 추상화 부재.

**Tailscale**: Rust 전용 공식 크레이트 없음. 환경변수(TAILSCALE_IP)를 감지하거나 `std::net` + tailscale 데몬 소켓 경유 방식으로 처리. 별도 크레이트 불필요.

---

### 6. UI

**권고: `ratatui` + `crossterm`**

`ratatui` v0.30.0 (2025-12-26). MIT. 다운로드 2570만+. tui-rs fork, 활발히 유지보수 중.

`crossterm` v0.29.0 (2025-04-05). MIT. 다운로드 1.26억+. 크로스 플랫폼 터미널 IO. ratatui의 기본 백엔드.

`tauri` v2.11.0 (2026-04-30). MIT OR Apache-2.0. Phase 2+ GUI용. Phase 1에서는 제외.

---

### 7. Lifecycle

**권고: `keyring` + `directories` + `tokio-cron-scheduler` + `trash`**

`keyring` v4.0.0 (2026-04-26). MIT OR Apache-2.0. macOS Keychain, Linux libsecret/KWallet, Windows Credential Manager 통합. 시크릿 키 보관에 적합.

`directories` v6.0.0 (2025-01-12). MIT OR Apache-2.0. XDG Base Directory 준수. 설정/데이터 경로 표준화.

`tokio-cron-scheduler` v0.15.1 (2025-10-28). MIT OR Apache-2.0. cron 표현식 기반 주기 작업.

`trash` v5.2.5 (2025-10-25). MIT OR Apache-2.0. 크로스 플랫폼 휴지통. rm 대신 사용.

---

### 8. 보안

**권고: `argon2` + `chacha20poly1305` + `zeroize`**

`argon2` v0.5.3 stable (2024-01-20). MIT OR Apache-2.0. Argon2id KDF. 주의: v0.6.0-rc.8까지 진행 중이며 stable이 2024년에 멈춰 있다. RustCrypto 전반이 0.6 RC 단계이므로 stable 0.5.3 사용 권고.

`chacha20poly1305` v0.10.1 stable. Apache-2.0 OR MIT. AES-GCM 대비 하드웨어 AES 가속 없는 환경에서 성능 우위. 모바일/임베디드 타겟 포함 시 권고.
`aes-gcm` v0.10.3 stable. Apache-2.0 OR MIT. 하드웨어 AES 가속 환경(x86-64 AES-NI)에서 선택. 둘 중 하나만 선택하면 된다.

`zeroize` v1.8.2 (2025-09-29). Apache-2.0 OR MIT. 다운로드 4.4억+. 메모리 zeroize 필수. k256·argon2 등 RustCrypto 크레이트가 내부적으로 이미 사용하므로 API 통일성을 위해 동일 버전을 명시적으로 포함.

---

### 9. 코드 추출 LLM 어댑터

**권고: reqwest 기반 직접 구현 (어댑터 패턴)**

Gemini, Anthropic, OpenAI, Ollama 모두 공식 Rust SDK가 없거나 비공식 상태다. 각 제공자의 REST API를 reqwest + serde_json으로 직접 호출하는 thin wrapper를 trait 기반 어댑터로 구현하는 것이 가장 안정적이다. 예:

```
trait LlmAdapter: Send + Sync {
    async fn extract_code(&self, prompt: &str) -> anyhow::Result<String>;
}
```

이 방식은 silent error를 강요하지 않으며, 각 provider별 HTTP 오류를 thiserror로 명시적으로 모델링할 수 있다.

Ollama는 `http://localhost:11434/api/generate` REST 엔드포인트만 있으면 충분하다.

---

### 10. 직렬화·로깅·기타

모두 생태계 표준이며 이견 없다.

`serde` v1.0.228 (2025-09-27). MIT OR Apache-2.0. 다운로드 9.7억+.
`serde_json` v1.0.149 (2026-01-06). MIT OR Apache-2.0. 다운로드 8.7억+.
`tracing` v0.1.44 (2025-12-18). MIT. 다운로드 5.7억+. tracing-subscriber 함께 사용.
`clap` v4.6.1 (2026-04-15). MIT OR Apache-2.0. 다운로드 8억+.
`anyhow` v1.0.102 (2026-02-20). MIT OR Apache-2.0. 다운로드 6.6억+. 빠른 프로토타이핑용 오류 처리.
`thiserror` v2.0.18 (2026-01-18). MIT OR Apache-2.0. 다운로드 9.5억+. 라이브러리 크레이트의 명시적 에러 타입 정의.

권고: 바이너리(main 크레이트)는 anyhow, 라이브러리 크레이트는 thiserror를 사용해 silent error 패턴을 원천 차단한다.

---

## 2. 우려 사항

### XMTP Rust SDK 미성숙 (높음)
공식 XMTP Rust 크레이트가 crates.io에 없다. crates.io의 `xmtp` v0.9.3은 비공식 개인 래퍼(731 다운로드)다. 공식 `libxmtp`는 Rust로 작성되었으나 WASM/Node/Swift/Kotlin 바인딩만 공개 배포한다. Phase 1에서 XMTP를 핵심 메시징 레이어로 쓰려면 reqwest 기반 HTTP 어댑터를 자체 구현하거나, libxmtp를 git 의존성으로 직접 참조해야 한다. 이는 빌드 재현성과 업스트림 API 변경 위험을 수반한다.

### k256 stable 버전 갭 (중간)
k256 안정 버전이 0.13.4(2024-09-20)이며 0.14.0-rc.9가 7개월째 RC 단계다. alloy-signer 2.x가 k256 0.14.x를 요구할 수 있어 Cargo.lock에서 버전 충돌이 발생할 가능성이 있다. 실제 통합 전에 Cargo tree로 확인 필요.

### ort 버전 분열 (중간)
`ort` stable 1.16.3과 RC 2.0.0 사이에 API 호환성이 없다(완전 재설계). fastembed 5.x가 ort 2.x RC를 의존한다면 전이적으로 불안정한 RC 크레이트가 빌드에 포함된다. fastembed의 Cargo.toml을 직접 확인하여 어느 ort 버전을 사용하는지 검증 권고.

### RustCrypto 0.x→0.6 RC 전반 (낮음)
argon2 0.5.3 / aes-gcm 0.10.3 / chacha20poly1305 0.10.1이 모두 stable이지만 마지막 stable 릴리스가 2022~2024년이다. RC 0.6이 장기간 진행 중인 것은 RustCrypto 전반의 패턴으로, 실제 사용에서 문제가 된 사례는 없으나 Phase 2 이후 마이그레이션 비용 존재.

### sqlite-vec alpha 단계 (낮음)
v0.1.9가 stable이지만 v0.1.10-alpha.3까지 진행 중이며 전체가 0.1.x 수준이다. API는 안정적이나 프로덕션 고부하 환경에서 검증 사례가 적다. Phase 1 MVP 수준에서는 충분하다.

---

## 3. Phase 1 의존성 트리 (추정)

핵심 직접 의존성 수: 약 22개
- crypto 스택(k256, bip39, coins-bip32, alloy-signer): ~15 transitive
- DB 스택(rusqlite, sqlite-vec): ~5 transitive (sqlite-vec는 cc로 C 소스 정적 링크, Rust 의존성 없음)
- 임베딩(fastembed): ~30 transitive (ort + ONNX Runtime C 라이브러리 포함)
- 네트워크(tokio, reqwest, axum): ~25 transitive
- 어댑터(serenity, teloxide): ~20 transitive 각각
- 보안(argon2, chacha20poly1305, zeroize): ~10 transitive (RustCrypto 내부 공유)
- 직렬화·로깅(serde, tracing, clap, anyhow, thiserror): ~10 transitive

전체 추정 transitive 의존성: 100~140개

---

## 4. 빌드 시간 예상 (clean build)

개발 머신(M-series or Ryzen 7급) 기준:

- 첫 clean build: 4~7분
  - fastembed + ort가 ONNX Runtime C 라이브러리를 cmake로 빌드하면 최대 10분+가 될 수 있음
  - ONNX Runtime 사전 설치(ORT_DYLIB_PATH 환경변수) 시 fastembed 빌드 시간 대폭 단축
- 증분 빌드 (코드 1~2파일 변경): 10~30초
- `cargo check` (컴파일 없이 타입 체크): 5~15초

빌드 시간 최적화 권고사항:
- mold 링커 사용 (Linux)
- fastembed의 dynamic linking feature 활성화 (ORT_DYLIB_PATH 사용)
- CI에서 sccache 캐시 적용

---

## 5. 마스터 결정 필요한 Trade-off

### Trade-off 1: DB 드라이버 — rusqlite(sync) vs sqlx(async)
- rusqlite: sqlite-vec 연동 직관적, sync 블로킹, tokio spawn_blocking 필요
- sqlx: async 자연스럽지만 sqlite-vec 확장 로딩이 번거롭고 0.9.0-alpha.1만 최신
- 권고: Phase 1에서 rusqlite. sqlite-vec 연동 단순성 우선.

### Trade-off 2: XMTP 통합 방식
- 옵션 A: reqwest로 XMTP HTTP REST API 직접 호출 (단순, XMTP 기능 일부만)
- 옵션 B: libxmtp git 의존성으로 포함 (전체 MLS 기능, 빌드 복잡도↑, 업스트림 변경 리스크)
- 옵션 C: Phase 1에서 XMTP 제외, 자체 E2E 암호화 메시지 레이어 구현
- 결정 필요: XMTP의 MLS group messaging이 Phase 1 MVP에 필수인지 여부.

### Trade-off 3: 임베딩 모델 — BGE-small(영어) vs multilingual-e5-small(다국어)
- fastembed는 모두 지원하지만 한국어 메시지를 임베딩한다면 multilingual-e5-small 필수
- 모델 파일 크기: BGE-small ~130MB, multilingual-e5-small ~560MB
- 결정 필요: Phase 1에서 한국어 검색/유사도가 필요한지 여부.

### Trade-off 4: 대칭 암호화 — AES-GCM vs ChaCha20-Poly1305
- aes-gcm: x86-64 AES-NI 가속 필수 (하드웨어 없으면 느림)
- chacha20poly1305: 모든 환경 균일한 성능, 하드웨어 가속 불필요
- 결정 필요: 배포 타겟에 ARM/임베디드/모바일이 포함되는지 여부.

---

## 6. Silent Error 패턴 주의 크레이트

마스터의 fallback 금지 원칙에 따라 아래를 명시한다.

- `reqwest`: Response::error_for_status()를 호출하지 않으면 HTTP 4xx/5xx가 Ok()로 반환된다. 모든 HTTP 호출 후 `.error_for_status()?`를 강제하는 래퍼 함수를 공통화해야 한다.
- `rusqlite`: `execute()` 반환값(영향받은 행 수)을 무시하면 UPDATE/DELETE가 0행에 적용되어도 오류가 발생하지 않는다. affected rows 검증 로직 필수.
- `tokio-cron-scheduler`: job 실행 중 panic이 발생해도 스케줄러가 계속 실행된다. job 내부를 catch_unwind로 감싸거나 tracing error로 명시적 로깅 필요.
- `keyring`: 일부 Linux 환경(headless server)에서 keychain 백엔드 없이 Ok(())를 반환하며 실제로는 저장하지 않는 구현이 존재했음(과거 이슈). Phase 1에서 저장 직후 read-back 검증 권고.

---

## 권고 요약표 (텍스트)

1. 키페어: k256 0.13.4 + bip39 2.2.2 + coins-bip32 0.13.0 + alloy-signer 2.0.4
2. DB: rusqlite 0.39.0 + sqlite-vec 0.1.9
3. 임베딩: fastembed 5.13.4 (BGE-small/multilingual-e5 선택)
4. 네트워크: tokio 1.52.1 + reqwest 0.13.3 + axum 0.8.9
5. XMTP: reqwest 기반 자체 HTTP 어댑터 (공식 Rust 크레이트 없음)
6. Discord: serenity 0.12.5
7. Telegram: teloxide 0.17.0
8. TUI: ratatui 0.30.0 + crossterm 0.29.0
9. Lifecycle: keyring 4.0.0 + directories 6.0.0 + tokio-cron-scheduler 0.15.1 + trash 5.2.5
10. 보안: argon2 0.5.3 + chacha20poly1305 0.10.1 + zeroize 1.8.2
11. 직렬화·로깅: serde 1.0.228 + serde_json 1.0.149 + tracing 0.1.44 + clap 4.6.1 + anyhow 1.0.102 + thiserror 2.0.18
12. LLM 어댑터: reqwest 기반 trait 어댑터 (Gemini/Anthropic/OpenAI/Ollama 공통)
