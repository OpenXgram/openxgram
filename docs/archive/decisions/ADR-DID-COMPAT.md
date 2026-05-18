# ADR — W3C DID + 한국 OpenDID + OmniOne Open DID 호환

- 상태: Accepted
- 일시: 2026-05-04 (KST)
- 결정자: master
- 구현: `crates/openxgram-did`, `crates/openxgram-cli` (`xgram identity`)

## 결정

OpenXgram master secp256k1 키페어를 그대로 W3C DID 로 노출하고, 한국디지털인증협회 OpenDID(opendid.org) + OmniOne Open DID(opendid.omnione.net) 두 한국 표준에도 매핑한다. 별도 키 발급·등록 절차 없음.

## 동기

- OpenXgram 은 master 키페어를 이미 운영 중 — 같은 키가 W3C DID 표면을 가지면 외부 시스템(SSO, 정부 신분증, 기업 IdP) 호환이 즉시 확보됨.
- 한국 시장 진입 시 두 표준(opendid.org / opendid.omnione.net) 호환은 사실상 필수.
- 추가 키 인프라를 두면 5층 메모리·vault 와 어긋남 → 동일 master 만 사용.

## 표준 매핑

- did:key (W3C did-method-key): `did:key:z` + base58btc(varint[0xe7, 0x01] || 33B compressed secp256k1 pubkey)
- secp256k1-pub multicodec: `0xe7`
- DID Document verificationMethod type: `EcdsaSecp256k1VerificationKey2019` (W3C Security Vocab)
- VC proof type: `EcdsaSecp256k1Signature2019`, `proofPurpose: assertionMethod`, JWS detached
- 한국 OpenDID(opendid.org) 매핑: `did:opendid:{network}:{base58btc(SHA-256(pubkey))[..22]}`
- OmniOne Open DID(opendid.omnione.net) 매핑: `did:omn:{base58btc(SHA-256(pubkey))[..22]}`

## 근거 — 의사결정 기록

- W3C DID Core 1.0: <https://www.w3.org/TR/did-core/>
- W3C did:key Method (CCG): <https://w3c-ccg.github.io/did-method-key/>
- W3C VC Data Model 1.1: <https://www.w3.org/TR/vc-data-model/>
- multiformats multicodec table — secp256k1-pub = 0xe7
- 한국디지털인증협회: <https://opendid.org/technical/did.php>
- OmniOne Open DID: <https://opendid.omnione.net/community/about>, <https://github.com/OmniOneID/did-doc-architecture>
- 한국 OpenDID 공식 method 표기 미공개 — 보수적으로 `did:opendid:{network}` 매핑 채택. 협회 공식 method 가 공개되면 alias 추가로 흡수.

## 대안 비교

- 별도 DID 키 발급: master 키와 분리 → 5층 메모리·vault·audit chain 과 어긋남. 채택 안 함.
- did:web 만 지원: SSO 호환은 되나 한국 시장 매핑 부족. 채택 안 함.
- did:key 만 지원: W3C 만 만족, 한국 표준 미지원. 마스터 지시("둘 다 반영") 미충족. 채택 안 함.

## 영향

- 새 crate `openxgram-did` 추가 (workspace member).
- CLI `xgram identity {did|did-document|issue-vc|verify-vc}` 추가.
- DB·마이그레이션 변경 없음 — DID 는 master pubkey 에서 derived.
- vault/audit chain 영향 없음.

## 검증

- 단위 테스트: `cargo test -p openxgram-did` (6개 통과 — did:key 인코딩, JSON-LD 필수 필드, opendid-kr/omnione 형식, VC 라운드트립, 변조 검출).
- 통합 테스트: `crates/openxgram-did/tests/round_trip.rs` (full pipeline 1개 통과).
- CLI 실 동작: master 키 → did:key + opendid-kr + omnione 출력 + DID Document JSON-LD + VC 발급/검증 라운드트립 확인 완료.

## 절대 규칙 준수

- fallback 금지: 모든 오류 `DidError`/`anyhow` raise, silent skip 없음.
- DB 변경 없음: 마이그레이션 0.
- 시간대 KST: VC `issuanceDate` 는 RFC3339 UTC `Z` (W3C 표준 강제) — 표시·로그 KST.
- 표 사용 없음: 본 ADR 모든 항목 목록.
