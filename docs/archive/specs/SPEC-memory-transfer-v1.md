# SPEC — Memory Transfer (MT) Module v1

작성일: 2026-04-30 (KST)
버전: v0.1.0.0-alpha.1
상태: 초안
작성자: Pip (agt_dept_prd)
기반: PRD-OpenXgram-v1.md (§10, §18), 마스터 누적 결정

---

## 1. 개요

### 1.1 한 줄 정의

"사이드카와 웹 LLM/외부 시스템 간 양방향 기억 전이"

### 1.2 본질

MT 모듈은 OpenXgram 사이드카가 자신이 보유한 기억(L0~L4)과 결정을 외부로 꺼내거나 외부에서 가져오는 진입점이다.
웹 LLM(ChatGPT, Claude Web 등)은 사이드카 프로토콜을 직접 구현하지 않으므로, 사람이 중간 다리가 되어 복사-붙여넣기로 기억을 주고받는다. 이것이 push-only 1등 시민 원칙이다.
외부 시스템(Linear, Notion, 자체 서버)은 webhook endpoint를 통해 자동으로 기억을 사이드카에 밀어 넣거나 꺼낼 수 있다.

### 1.3 핵심 시나리오 5개

시나리오 A — ChatGPT 웹과 토론 후 사이드카로 import
- 마스터가 ChatGPT 웹에서 아키텍처 결정을 마무리한다
- `xgram extract --format text-package` 로 현재 세션 요약을 추출한다
- 마스터가 텍스트 패키지를 클립보드에 복사해 ChatGPT 창에 붙여넣는다
- ChatGPT가 검토·보강한 내용을 마스터가 다시 클립보드로 복사한다
- `xgram session import --from clipboard` 로 사이드카에 들어온다
- 결과: 사이드카의 현재 세션에 ChatGPT 대화 내용이 L0 메시지로 추가되고 L2 결정이 자동 추출된다

시나리오 B — 사이드카 → Notion에 결정 자동 export (webhook)
- 마스터가 `xgram webhook add --name notion-decisions --transport http-post --target https://hook.notion.so/...` 를 설정한다
- 이후 `pin` 이벤트 발생 시 webhook 트리거가 자동 발화한다
- Notion 측은 HMAC 서명을 검증하고 페이지를 업데이트한다
- 결과: 마스터가 별도 조작 없이 핀된 결정이 Notion에 실시간 반영된다

시나리오 C — Cloudflare 설정을 코드 형태로 추출
- 마스터가 `xgram extract --format code --lang python --tag cloudflare` 를 실행한다
- MT 모듈이 vault와 L2 memories에서 `cloudflare` 태그를 가진 항목을 수집한다
- Gemini API 호출로 수집된 항목을 Python Cloudflare API 스크립트로 변환한다
- 결과: 실행 가능한 `.py` 파일이 생성되고 audit_log에 기록된다

시나리오 D — 외부 시스템(Linear)이 사이드카에 작업 결과 inbound
- Linear webhook이 이슈 완료 이벤트를 사이드카 inbound endpoint `POST /xgram/inbound` 로 전송한다
- MT 모듈이 HMAC 검증 + 화이트리스트 확인 후 페이로드를 파싱한다
- L0 메시지로 저장되고 `issue-complete` 태그 L2 memory가 생성된다
- 결과: 사이드카가 Linear 작업 흐름을 자동으로 기억하고 회상에 활용한다

시나리오 E — Discord에 매일 18시 자동 백업
- 마스터가 `xgram backup-push --channel discord --cron "0 18 * * *"` 를 설정한다
- 매일 18시 KST에 당일 L2 memories와 핀된 항목을 Markdown 파일로 추출한다
- `#xgram-backup` 채널에 파일 첨부와 함께 전송된다
- 결과: 사이드카 백업이 Discord에 일별로 보관된다

---

## 2. 용어

### 2.1 Push / Pull / Transfer

- Push — 사이드카가 외부로 기억을 내보내는 방향. outbound 라고도 부른다. 마스터 승인 정책이 적용된다.
- Pull — 외부에서 사이드카로 기억을 가져오는 방향. inbound 라고도 부른다. HMAC 검증과 화이트리스트가 적용된다.
- Transfer — Push와 Pull을 모두 포함하는 상위 개념. MT 모듈 전체를 가리킨다.

### 2.2 Outbound / Inbound

- Outbound — 사이드카가 발신자가 되어 외부 채널 또는 시스템으로 데이터를 보내는 흐름.
- Inbound — 외부 시스템이 발신자가 되어 사이드카 HTTP endpoint로 데이터를 밀어넣는 흐름.

### 2.3 추출 형식 4종

- Text Package — Markdown 헤더 + 사람이 읽을 수 있는 요약 + 기계가 처리할 수 있는 JSON 본체를 하나의 블록으로 묶은 형식. 웹 LLM에 붙여넣기 최적화.
- File — 단일 파일로 직렬화한 형식. `.md`, `.json`, `.yaml` 중 하나. 이메일 첨부나 로컬 아카이브 용도.
- Code — 메모리를 실행 가능한 코드로 변환한 형식. Python, TypeScript, SQL, Nginx conf, JSON config, Bash 지원. 변환은 Gemini API 경유.
- Webhook Payload — 서명이 포함된 JSON envelope. 외부 시스템에 자동 전달되는 구조화 데이터.

### 2.4 백업 채널 3종

- Clipboard — 클립보드에 텍스트를 복사. 수동, 즉각, 인증 불필요.
- Telegram — @starianbot을 통해 마스터 chat_id로 전송. 4096자 제한 자동 분할.
- Discord — 봇 또는 webhook을 통해 지정 채널에 전송. 4000자 제한 처리.

이메일(SMTP) 어댑터는 영구 제외되었다. 마스터 결정(2026-04-30).

### 2.5 1등 시민 vs Push-only 시민 vs 외부 시스템

- 1등 시민 — xgram 프로토콜을 완전히 구현한 사이드카 데몬. IPC / Tailscale / XMTP 자동 라우팅 모두 사용 가능.
- Push-only 시민 — 웹 LLM(ChatGPT, Claude Web 등). 사이드카 프로토콜 없음. 사람이 복사-붙여넣기 다리 역할을 해서 기억을 주고받는다. outbound는 가능하지만 자동 inbound 불가.
- 외부 시스템 — webhook endpoint를 통해 사이드카와 통신하는 서드파티(Linear, Notion, 자체 서버 등). HMAC 인증 기반 자동화 가능.

---

## 3. 워크플로우

### 3.1 Push (Send Out)

#### Step 1: 범위 선택

선택 가능한 범위 옵션:
- `--all` — 전체 세션의 모든 메시지와 기억
- `--last N` — 최근 N개 메시지 (기본값: 50)
- `--since Nh` — 최근 N시간 이내 (기본값: 24h)
- `--pinned-only` — 핀된 L2 memories만
- `--tag TAG` — 특정 태그가 붙은 항목만 (복수 가능: `--tag a --tag b`)
- `--search QUERY` — 임베딩 검색으로 관련 항목 추출 (top-K 기본 20)
- `--session SESSION_ID` — 특정 세션 ID

기본값: `--last 50 --since 24h` 교집합. 권장값: 목적에 맞는 `--tag` 또는 `--pinned-only` 명시.

#### Step 2: 형식 선택

- `--format text-package` — 기본값. 웹 LLM 붙여넣기 목적. Markdown + JSON 블록 결합. 예상 크기: 10~50 KB.
- `--format file --ext md|json|yaml` — 로컬 저장 또는 이메일 첨부 목적. 예상 크기: 1~100 KB.
- `--format code --lang LANG` — 코드 변환 목적. Gemini API 호출 포함. 예상 크기: 1~10 KB.
- `--format webhook` — 외부 자동화 목적. JSON envelope + 서명. 예상 크기: 5~500 KB.

형식별 사용 케이스와 예시는 4절에 상세 기술.

#### Step 3: 대상 선택

- `--target clipboard` — 클립보드 복사. 추가 설정 불필요. 보안: 로컬 전용.
- `--target email` — SMTP 발송. 사전 설정 필요 (5.2절). 보안: TLS 필수.
- `--target telegram` — @starianbot 경유. chat_id 필요 (5.3절). 보안: 봇 토큰.
- `--target discord` — 봇 또는 webhook URL 경유. 채널 ID 필요 (5.4절). 보안: 봇 토큰 또는 webhook URL.
- `--target URL` — Custom webhook. URL + HMAC 시크릿 필요. 보안: HMAC-SHA256.

자동화 정도 순서: URL (최고) > discord > telegram > email > clipboard (수동).

#### Step 4: 보안 검증

- 태그 자동 제외: `secret`, `vault`, `private`, `internal` 태그 항목은 payload에서 자동 제거.
- 키 패턴 자동 마스킹: API 키 패턴, hex 토큰, BIP39 시드 단어를 정규식으로 탐지해 `[REDACTED]` 치환.
- 미리보기: `--preview` 플래그로 실제 발송 전 stdout에 payload 출력. dry-run과 동일.
- Rate limit: 시간당 최대 10회 outbound, 일당 최대 50회. 초과 시 즉시 raise.
- 승인 정책: `auto` (기본, 저위험) / `confirm` (마스터 TUI/Discord 확인) / `mfa` (추가 인증).

#### Step 5: 발송

- 승인 정책 `auto`: 즉시 발송.
- 승인 정책 `confirm`: TUI 또는 Discord 메시지로 마스터 확인 요청. 타임아웃 5분. 미응답 시 취소.
- 승인 정책 `mfa`: 추가 인증 토큰 입력 요구.
- 발송 완료 후 `transfer_logs` 테이블에 기록 (9절 스키마 참조).
- 응답 코드 200: 성공으로 기록. 4xx/5xx: 즉시 raise, fallback 금지.

---

### 3.2 Pull (Receive)

#### Step 1: 입력 방식

- 클립보드 붙여넣기: `xgram session import --from clipboard`. 마스터가 외부 LLM 대화를 복사한 뒤 실행.
- 파일 드래그앤드롭: GUI(Phase 2)에서 `.md`, `.json`, `.yaml` 파일을 드래그앤드롭.
- 파일 경로 직접 지정: `xgram session import --file PATH`.
- HTTP POST 수신: inbound webhook endpoint `POST /xgram/inbound` 로 외부 시스템이 자동 전송.

#### Step 2: 파싱·검증

- 형식 자동 감지: 입력 첫 줄 휴리스틱으로 markdown / JSON / yaml 자동 판별. 판별 실패 시 raise.
- 서명 검증: Webhook Payload 형식이면 HMAC-SHA256 서명 필수 검증. 서명 불일치 즉시 raise.
- 스키마 검증: JSON Schema 기반. 필수 필드 누락 시 raise. 잉여 필드는 무시.
- 페이로드 크기: 1 MB 상한. 초과 시 즉시 raise.

#### Step 3: 대상 세션

- `--new` — 새 세션 생성 (기본값).
- `--current` — 현재 열린 세션에 병합.
- `--session SESSION_ID` — 지정된 세션에 추가.

#### Step 4: 메모리 분류

- L0 messages — 원시 메시지 단위 항목이 직접 삽입됨. 역할(role) 필드 필수.
- L1 episodes — 세션 단위 묶음. 새 세션 생성 시 자동 생성. 병합 시 기존 에피소드에 편입.
- L2 memories — `type: decision|fact|rule|reference` 명시 항목은 L2로 직접 삽입. 그 외는 야간 reflection에서 자동 추출.
- L3, L4 — inbound로 직접 삽입 불가. reflection 전용.

#### Step 5: 충돌 처리

- 중복 메시지 감지: 서명 hash 기준으로 동일 항목이 이미 존재하면 skip (오류 아님).
- 시간 역순 처리: inbound 메시지의 timestamp가 기존 메시지보다 과거이면 올바른 위치에 삽입. 인덱스 재정렬.
- 충돌 로그: `transfer_logs` 에 `status=conflict-skipped` 로 기록.

#### Step 6: 임베딩 생성

- 기본값: `--embed auto` — 삽입 즉시 multilingual-e5-small로 임베딩 생성. 한국어/영어 모두 처리. fastembed 통합. 모델 크기 560MB. (마스터 결정 2026-04-30)
- `--embed defer` — 야간 reflection 시 일괄 생성.
- 임베딩 실패 시 즉시 raise. 조용히 넘어가지 않음. fallback 금지.

---

## 4. 추출 형식 상세 사양

### 4.1 Text Package

#### 정의

사람이 웹 LLM 창에 그대로 붙여넣을 수 있도록 최적화된 혼합 형식이다. 상단은 사람이 읽을 Markdown 요약, 하단은 기계가 처리할 JSON 본체로 구성된다. LLM이 JSON 블록을 보고 구조화된 기억을 흡수할 수 있다.

#### 스키마

```
---xgram-text-package-start---
# [Summary]
<자유 형식 Markdown 요약>

## Decisions
- <decision 1>
- <decision 2>

## Context
<배경 설명>

---xgram-json-start---
{
  "version": "1",
  "exported_at": "<ISO 8601 KST>",
  "sender": "<secp256k1 주소>",
  "session_id": "<uuid>",
  "messages": [ { "role": "...", "content": "...", "timestamp": "..." } ],
  "memories": [ { "type": "decision|fact|rule|reference", "content": "...", "tags": [] } ]
}
---xgram-json-end---
---xgram-text-package-end---
```

#### 예시

```
---xgram-text-package-start---
# OpenXgram MT 모듈 설계 결정

2026-04-30 마스터와 MT 모듈 구조 확정.

## Decisions
- fallback 금지: 모든 검증 실패는 raise
- Outbound 자동 승인 금지: 명시 승인만

## Context
OpenXgram 사이드카의 Memory Transfer 모듈 초기 설계.

---xgram-json-start---
{
  "version": "1",
  "exported_at": "2026-04-30T18:00:00+09:00",
  "sender": "0xABC...123",
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "messages": [
    { "role": "user", "content": "fallback 금지 규칙 확정해줘", "timestamp": "2026-04-30T17:55:00+09:00" }
  ],
  "memories": [
    { "type": "rule", "content": "모든 검증 실패는 raise. fallback 금지.", "tags": ["mt", "security"] }
  ]
}
---xgram-json-end---
---xgram-text-package-end---
```

- 크기 가이드: 일반 세션 10~30 KB, 대형 세션 최대 500 KB. 500 KB 초과 시 경고 표시.
- 사용 케이스: 웹 LLM 대화에서 기억 추출 후 사이드카로 재흡수. 마스터가 다리 역할.
- 보안 고려사항: `secret`/`vault` 태그 항목 자동 제외. 발송 전 `--preview`로 내용 확인 권장.

---

### 4.2 단일 파일

#### 정의

하나의 파일로 직렬화된 형식. 로컬 아카이브, 이메일 첨부, 버전 관리에 적합하다.

#### 스키마 (메타데이터 frontmatter 공통)

```yaml
---
xgram_version: "1"
format: "file"
ext: "md|json|yaml"
exported_at: "<ISO 8601 KST>"
sender: "<secp256k1 주소>"
session_id: "<uuid>"
tags: []
---
```

지원 형식별 본문 구조:
- `.md` — frontmatter + Markdown 본문. 메시지는 대화 형식, 기억은 목록.
- `.json` — frontmatter 포함 JSON 단일 객체. `messages`, `memories`, `metadata` 배열.
- `.yaml` — frontmatter + YAML 본문. JSON과 동일 구조, 가독성 우선.

#### 예시 (`.md`)

```markdown
---
xgram_version: "1"
format: "file"
ext: "md"
exported_at: "2026-04-30T18:00:00+09:00"
sender: "0xABC...123"
session_id: "550e8400-e29b-41d4-a716-446655440000"
tags: ["mt", "design"]
---

## Session: OpenXgram MT 설계

**user**: fallback 금지 원칙 확정  
**assistant**: 확인. 모든 검증 실패는 raise입니다.

## Memories

- [rule] 모든 검증 실패는 raise. fallback 금지.
- [decision] outbound 자동 승인 금지.
```

- 크기 가이드: 1~100 KB (세션 길이에 비례).
- 사용 케이스: 이메일 첨부, 로컬 백업, Git 이력 보관.
- 보안 고려사항: 파일 권한 600 권장 (소유자만 읽기). 이메일 전송 시 TLS 필수.

---

### 4.3 코드 추출

#### 4.3.1 LLM 어댑터 인터페이스

코드 변환은 단일 LLM에 고정하지 않는다. 사용자가 어댑터를 선택한다. 마스터 결정(SPEC 16절 확정, 2026-04-30).

어댑터 trait: `CodeExtractor`
- `extract(memory: &Memory, lang: Lang, hints: &Hints) -> Code`

구현체:
- `GeminiAdapter` — Google Gemini API (`data-api.starian.us` 또는 직접 호출)
- `ClaudeAdapter` — Anthropic API
- `OpenAIAdapter` — OpenAI GPT-4
- `OllamaAdapter` — 로컬 모델 (Ollama HTTP API)
- `TemplateAdapter` — LLM 없이 템플릿 기반 변환 (오프라인 환경)

사용자 선택: `~/.openxgram/config.toml`의 `[code_extractor]` 섹션

```toml
[code_extractor]
default_adapter = "gemini"   # 기본 어댑터

[code_extractor.per_lang]
python = "gemini"
sql    = "claude"
bash   = "template"
```

per-language 오버라이드 가능. 어댑터 미설정 시 `default_adapter` 사용. 강제 없음.

어댑터 API 키는 vault에 저장. `xgram config set code_extractor.gemini_key_vault_key gemini_api_key` 방식으로 등록.

#### 정의

메모리에서 추출한 설정·결정을 실행 가능한 코드로 변환한다. multilingual-e5-small은 임베딩 전용이므로 실제 변환은 4.3.1에서 선택된 LLM 어댑터를 호출한다.

#### 지원 언어

- Python — Cloudflare API, 인프라 자동화, 데이터 처리
- TypeScript — 프론트엔드 설정, API 클라이언트
- SQL — DB 마이그레이션, 쿼리
- Nginx conf — 서버 설정
- JSON config — 애플리케이션 설정
- Bash — 셸 스크립트, cron 작업

#### 변환 규칙

메모리 종류별 코드 형태 매핑:
- `type: decision` + `tag: cloudflare` → Python Cloudflare API 스크립트
- `type: rule` + `tag: nginx` → Nginx conf 블록
- `type: decision` + `tag: db` → SQL DDL 또는 마이그레이션 스크립트
- `type: fact` + `tag: config` → JSON config 파일
- `type: rule` + `tag: cron` → Bash + crontab 항목
- `type: decision` + `tag: typescript|ts` → TypeScript 모듈

변환 흐름:
1. 지정 태그로 L2 memories와 vault 항목 검색
2. 수집된 항목을 Gemini API에 전달 (변환 프롬프트 포함)
3. 반환된 코드 블록 파싱 + 파일로 저장
4. audit_log에 `format=code, lang=<LANG>` 으로 기록

#### 예시 (Python, `--tag cloudflare`)

```python
# Generated by xgram extract --format code --lang python --tag cloudflare
# Session: 550e8400 | Exported: 2026-04-30T18:00:00+09:00
import CloudFlare

cf = CloudFlare.CloudFlare(token="[REDACTED]")
# Decision: A 레코드 oracle.starian.us → 34.x.x.x
cf.zones.dns_records.post(
    "ZONE_ID",
    data={"type": "A", "name": "oracle.starian.us", "content": "34.x.x.x", "proxied": True}
)
```

- 크기 가이드: 1~10 KB. 대형 설정 스크립트 최대 50 KB.
- 사용 케이스: 인프라 재현, 설정 자동화, 코드 리뷰.
- 보안 고려사항: API 키, 토큰은 항상 `[REDACTED]` 치환. 코드 실행 전 마스터 확인 필수.

---

### 4.4 Webhook Payload

#### 정의

서명이 포함된 JSON envelope. 외부 시스템이 서명을 검증해 payload의 무결성과 발신자를 확인할 수 있다.

#### 표준 envelope 구조

```json
{
  "version": "1",
  "signature": "<secp256k1 ECDSA 서명, hex>",
  "timestamp": "<ISO 8601 KST, 5분 이내 유효>",
  "sender_address": "<secp256k1 공개 주소>",
  "payload": {
    "type": "memory-transfer",
    "session_id": "<uuid>",
    "messages": [],
    "memories": [],
    "metadata": {}
  }
}
```

필수 필드:
- `version` — 항상 `"1"` (현재)
- `signature` — secp256k1 ECDSA. 서명 대상: `SHA256(version + timestamp + sender_address + JSON.stringify(payload))`
- `timestamp` — ISO 8601 KST. 수신 측은 현재 시각 기준 ±5분 이내만 허용.
- `sender_address` — 발신자 secp256k1 공개 주소. 화이트리스트 검증에 사용.
- `payload` — 실제 데이터. `type` 필드로 수신 측 파서 결정.

#### 서명 알고리즘

서명 알고리즘: secp256k1 ECDSA
서명 대상 문자열: `SHA256(version || timestamp || sender_address || canonical_json(payload))`
canonical JSON: 키 알파벳 정렬, 공백 없음.
서명 출력: DER 인코딩, hex 문자열.

#### 검증 절차 (수신 측)

1. `timestamp` 파싱 → 현재 시각 대비 ±5분 확인. 초과 시 즉시 reject.
2. `sender_address` 화이트리스트 확인. 목록에 없으면 즉시 reject.
3. 서명 대상 문자열 재구성 → secp256k1 서명 검증. 불일치 시 즉시 reject.
4. 페이로드 크기 1 MB 이하 확인. 초과 시 즉시 reject.
5. JSON Schema 검증. 실패 시 즉시 reject.
6. 모든 검증 통과 후 파싱·저장 진행.

---

## 5. 백업 채널 3종 + Discord 전송 사양

이메일(SMTP) 채널은 영구 제외되었다. 마스터 결정(2026-04-30). 백업 채널은 클립보드·Telegram·Discord 3종으로 확정.

### 5.1 클립보드

설정 방법: 별도 설정 불필요. `--target clipboard` 플래그만 사용.

전송 메커니즘:
- Linux: `xclip` 또는 `wl-copy` (Wayland) 시스템 명령 경유.
- macOS: `pbcopy` 시스템 명령 경유.
- headless 환경: `xvfb-run` 또는 파일 출력으로 fallback — 단, fallback 안내 메시지를 raise해야 함. 조용한 무시 금지.

인증 방식: 없음. 로컬 전용.

Rate limit: 없음.

실패 처리: 클립보드 도구 없음 → 즉시 raise + 필요 패키지 안내.

응답 회수 방법: 마스터가 외부 LLM 창에 붙여넣기 후, 응답 복사 → `xgram session import --from clipboard`.

TUI/GUI에서: `[C 복사]` 단축키로 현재 추출 결과를 클립보드에 바로 복사.

---

### 5.2 Telegram

설정 방법:

```bash
xgram config set telegram.bot_token_vault_key telegram_bot_token  # vault 키 참조
xgram config set telegram.chat_id 6565914284  # 마스터 chat_id
```

전송 메커니즘: Telegram Bot API `sendMessage` (텍스트) + `sendDocument` (파일). @starianbot 경유.

인증 방식: Bot 토큰. vault에 저장.

Rate limit: Telegram API 기본 초당 30개. MT 모듈 자체 제한: 분당 10회.

메시지 분할: 4096자 초과 시 자동 분할. 각 부분에 `[1/N]`, `[2/N]` 접두사 추가.

실패 처리: API 오류 → 즉시 raise. 자동 재전송 없음.

응답 회수 방법: 마스터가 Telegram에서 외부 LLM 응답을 포워드 → `xgram session import --from telegram` (Phase 2).

---

### 5.3 Discord

설정 방법:

봇 방식:
```bash
xgram config set discord.bot_token_vault_key discord_bot_token  # vault 키 참조
xgram config set discord.backup_channel_id CHANNEL_ID  # #xgram-backup 채널 ID
```

Webhook 방식 (채널별 설정):
```bash
xgram webhook add --name discord-backup --transport discord-webhook \
  --target WEBHOOK_URL --channel xgram-backup
```

전송 메커니즘: Discord REST API `POST /channels/{channel_id}/messages`. 파일은 multipart form-data 첨부.

인증 방식: 봇 토큰 (Authorization: Bot TOKEN) 또는 webhook URL.

Rate limit: Discord 기본 분당 5회/채널. MT 모듈 자체 제한: 분당 3회.

4000자 제한 처리: 4000자 초과 시 파일 첨부로 전환. Text Package는 `.md` 파일로 첨부.

채널 매핑:
- 기본 백업 채널: `#xgram-backup` (전용 채널, 결정 필요 항목 16절 참조)
- 세션별 채널: 에이전트 채널에 직접 보고 시 해당 채널 ID 사용

실패 처리: API 오류 → 즉시 raise. fallback 채널 자동 전환 금지.

응답 회수 방법: 백업 채널에서 메시지/첨부파일 복사 → `xgram session import --from clipboard` 또는 `--file`.

---

## 6. 양방향 Webhook 사양

### 6.1 Outbound Webhook

명령:
```bash
xgram extract --format webhook --target https://example.com/hook
xgram backup-push --channel webhook --target https://example.com/hook
```

HTTP 메서드: POST

헤더:
- `X-Xgram-Signature: <secp256k1 ECDSA 서명, hex>`
- `X-Xgram-Timestamp: <ISO 8601 KST>`
- `X-Xgram-Sender: <secp256k1 공개 주소>`
- `Content-Type: application/json`

페이로드: 4.4절 Webhook Payload 형식.

응답 처리:
- 200~299: 성공. `transfer_logs`에 `status=success` 기록.
- 4xx: 클라이언트 오류. 즉시 raise. 재시도 없음.
- 5xx: 서버 오류. 명시 retry 허용 (최대 3회, 지수 백오프 30s/60s/120s). 3회 실패 시 raise + 마스터 알림.
- silent retry 금지: 모든 retry는 로그에 기록.

재시도 정책:
- 5xx만 retry 대상.
- 최대 3회.
- 지수 백오프: 30초, 60초, 120초.
- 3회 모두 실패 시 `transfer_logs`에 `status=failed` + `error_message` 기록 후 raise.

---

### 6.2 Inbound Webhook

XMTP Transport 구현 참고: 공식 Rust SDK 부재(libxmtp는 WASM 우선 지원). XMTP 노드 REST API를 reqwest로 직접 호출한다. 모든 reqwest 호출에 `.error_for_status()?` 강제 적용. (마스터 결정 2026-04-30)

사이드카 HTTP API 엔드포인트:
- 경로: `POST /xgram/inbound`
- 포트: **14921** (확정. 14920은 다른 용도 예약. 마스터 결정 2026-04-30)

인증: HMAC-SHA256 + 시간 윈도우
- 검증 순서: timestamp ±5분 → 화이트리스트 → HMAC → Schema.
- HMAC 키: 엔드포인트 등록 시 생성된 공유 시크릿 (vault 보관).

페이로드 검증: 4.4절 Webhook Payload JSON Schema. 추가로 `payload.type` 필드가 등록된 타입인지 확인.

처리 결과 응답:
```json
{
  "status": "ok",
  "session_id": "<새로 생성된 세션 ID 또는 병합된 세션 ID>",
  "messages_inserted": 12,
  "memories_inserted": 3
}
```

오류 응답:
```json
{
  "status": "error",
  "code": "SIGNATURE_INVALID|TIMESTAMP_EXPIRED|WHITELIST_DENIED|SCHEMA_ERROR|PAYLOAD_TOO_LARGE",
  "message": "상세 오류 설명"
}
```

화이트리스트 정책:
- 기본값: **빈 목록** (모든 inbound 차단). 마스터가 `xgram webhook acl add` 명령으로 use 시점에 직접 추가. 마스터 결정(2026-04-30).
- 허용 방법: `xgram webhook acl add --address 0xABC...` 또는 `--ip 1.2.3.4` 또는 `--domain example.com`.
- 화이트리스트 미등록 요청: 즉시 403 + 마스터 알림.

---

## 7. UI/UX 사양

### 7.1 TUI 화면 (ratatui)

ASCII 와이어프레임:

```
┌─────────────────────────────────────────────────┐
│ OpenXgram — Memory Transfer            [ESC] 닫기 │
├─────────────────────────────────────────────────┤
│ [1] Push (Send Out)   [2] Pull (Receive)         │
│ [3] Webhook 관리      [4] 백업 채널              │
├─────────────────────────────────────────────────┤
│ PUSH                                             │
│ 범위:  [last 50] [pinned] [tag:___] [search:___] │
│ 형식:  [text-package] [file] [code] [webhook]    │
│ 대상:  [clipboard] [email] [telegram] [discord]  │
│                                                  │
│ 보안:  [secret 태그 자동 제외: ON] [마스킹: ON]  │
│ 승인:  [auto] [confirm] [mfa]                    │
│                                                  │
│ [P 미리보기]          [Enter 발송]               │
├─────────────────────────────────────────────────┤
│ 상태: 대기 중                                    │
│ 최근 발송: 2026-04-30 17:00:00 KST → clipboard  │
└─────────────────────────────────────────────────┘
```

키바인딩:
- `1~4` — 탭 전환
- `P` — 미리보기(--preview)
- `Enter` — 발송 실행
- `Tab` — 필드 이동
- `ESC` — MT 화면 닫기
- `C` — 클립보드 바로 복사 (현재 추출 결과)

상태 표시:
- `대기 중` — 발송 전
- `발송 중...` — HTTP 요청 진행
- `완료 (2026-04-30 18:00:00 KST)` — 성공
- `오류: SIGNATURE_INVALID` — 실패 (빨간색)

---

### 7.2 GUI 화면 (Tauri + React)

컴포넌트 구조:

```
MemoryTransferPage
├── TabBar (Push | Pull | Webhooks | Channels)
├── PushPanel
│   ├── ScopeSelector (체크박스 + 슬라이더)
│   ├── FormatSelector (라디오 버튼)
│   ├── TargetSelector (드롭다운)
│   ├── SecurityBadges (태그 제외 상태 표시)
│   ├── PreviewModal (미리보기 팝업)
│   └── SendButton + StatusBanner
├── PullPanel
│   ├── DropZone (드래그앤드롭 영역)
│   ├── ClipboardImportButton
│   ├── SessionTargetSelector
│   └── ParseResultPreview
├── WebhookManagePanel
│   ├── WebhookList
│   ├── AddWebhookForm
│   └── AclManageModal
└── ChannelConfigPanel
    ├── EmailConfig
    ├── TelegramConfig
    └── DiscordConfig
```

React 라우팅: `/memory-transfer` 경로. 사이드바 메뉴에 항목 추가.

드래그앤드롭 인터랙션:
- `.md`, `.json`, `.yaml` 파일을 `DropZone`에 드롭 → 자동 파싱 시작.
- 파싱 완료 전: 로딩 스피너.
- 파싱 완료 후: `ParseResultPreview`에 메시지 수, 기억 수, 충돌 수 표시.
- 마스터 확인 후 `[Import]` 버튼으로 삽입.

미리보기 모달:
- 실제 발송 payload를 모달에 전체 표시.
- `[복사]`, `[발송]`, `[취소]` 세 버튼.

---

### 7.3 CLI 명령어 전체 목록

#### xgram extract

설명: 메모리를 추출해 지정 형식으로 내보낸다.

인자:
- `--format text-package|file|code|webhook` — 추출 형식 (필수)
- `--target clipboard|email|telegram|discord|URL` — 발송 대상 (기본: clipboard)
- `--last N` — 최근 N개 메시지
- `--since Nh` — 최근 N시간
- `--pinned-only` — 핀된 항목만
- `--tag TAG` — 태그 필터 (반복 가능)
- `--search QUERY` — 임베딩 검색
- `--session SESSION_ID` — 특정 세션
- `--ext md|json|yaml` — 파일 형식 (format=file 시 사용)
- `--lang LANG` — 코드 언어 (format=code 시 사용)
- `--preview` — 미리보기 (발송 안 함)
- `--approve auto|confirm|mfa` — 승인 정책 오버라이드

예시:
```bash
xgram extract --format text-package --target clipboard --last 50
xgram extract --format file --ext md --target email --pinned-only
xgram extract --format code --lang python --tag cloudflare --preview
```

---

#### xgram backup-push

설명: 백업 채널로 현재 메모리 상태를 발송한다.

인자:
- `--channel clipboard|email|telegram|discord` — 채널 (필수)
- `--since Nh` — N시간 이내 변경 항목만 (기본: 24h)
- `--cron "CRON_EXPR"` — cron 스케줄 등록 (등록 후 자동 반복)

예시:
```bash
xgram backup-push --channel discord --since 24h
xgram backup-push --channel telegram --pinned-only
xgram backup-push --channel discord --cron "0 18 * * *"
```

---

#### xgram session import

설명: 외부 데이터를 현재 또는 지정 세션으로 가져온다.

인자:
- `--from clipboard|file|stdin` — 입력 방식 (필수)
- `--file PATH` — 파일 경로 (--from file 시 사용)
- `--new` — 새 세션 생성 (기본)
- `--current` — 현재 세션에 병합
- `--session SESSION_ID` — 지정 세션에 추가
- `--embed auto|defer` — 임베딩 생성 시점

예시:
```bash
xgram session import --from clipboard --new
xgram session import --from file --file ./backup.md --current
xgram session import --from stdin < exported.json
```

---

#### xgram webhook add

설명: webhook endpoint를 등록한다.

인자:
- `--name NAME` — 식별 이름 (필수)
- `--transport http-post|discord-webhook` — 전송 방식 (필수)
- `--target URL` — 대상 URL (필수)
- `--channel CHANNEL_NAME` — 채널 이름 (discord-webhook 시 사용)
- `--trigger pin|session-end|schedule|merge` — 트리거 이벤트 (반복 가능)
- `--cron "CRON_EXPR"` — schedule 트리거 시 cron 표현식

예시:
```bash
xgram webhook add --name notion-decisions --transport http-post \
  --target https://hook.notion.so/... --trigger pin
xgram webhook add --name discord-backup --transport discord-webhook \
  --target WEBHOOK_URL --cron "0 18 * * *"
xgram webhook add --name linear-inbound --transport http-post \
  --target http://localhost:14920/xgram/inbound
```

---

#### xgram webhook list

설명: 등록된 webhook 목록을 출력한다.

인자: 없음

예시:
```bash
xgram webhook list
```

---

#### xgram webhook test

설명: 등록된 webhook에 테스트 payload를 전송한다.

인자:
- `--name NAME` — 테스트할 webhook 이름 (필수)

예시:
```bash
xgram webhook test --name notion-decisions
```

---

#### xgram webhook remove

설명: 등록된 webhook을 삭제한다.

인자:
- `--name NAME` — 삭제할 webhook 이름 (필수)

예시:
```bash
xgram webhook remove --name linear-inbound
```

---

## 8. 보안 사양

### 8.1 Outbound 보안

태그 기반 자동 제외:
- 제외 태그: `secret`, `vault`, `private`, `internal`
- 해당 태그를 가진 L2 memory, L0 message 모두 payload에서 제거.
- 제외 여부는 `--preview` 출력에 `[제외됨: secret 태그]` 형태로 표시.

키 패턴 자동 마스킹:
- 탐지 패턴 목록:
  - API 키 패턴: `[A-Za-z0-9]{32,64}` (컨텍스트 기반)
  - hex 토큰: `0x[0-9a-fA-F]{40,}`
  - BIP39 시드: 12/24개 연속 영단어 패턴
  - 이메일 + 비밀번호 패턴
- 탐지 시 `[REDACTED:<type>]` 치환. 원본은 payload에 포함되지 않음.

미리보기 모드:
- `--preview` 플래그 또는 승인 정책 `confirm` 시 자동 활성화.
- 발송 전 마스터가 내용을 확인할 수 있는 유일한 단계.

마스터 승인 정책:
- `auto` — 저위험 작업 (클립보드 로컬 복사 등). 즉시 처리.
- `confirm` — Discord, Telegram 발송. TUI/Discord로 확인 요청.
- `mfa` — webhook 외부 발송. 추가 인증 토큰 입력.

Silent Error 방지 (reqwest):
- 모든 HTTP 응답에 `.error_for_status()?` 강제. 4xx/5xx를 `Ok`로 흡수하면 발송 실패를 탐지하지 못한다.
- XMTP REST, Discord webhook, Telegram API 등 모든 outbound reqwest 호출에 예외 없이 적용.

Rate limit:
- **분당 1000건** outbound. 안전망이며 정상 사용에는 영향 없음. fallback이 아닌 명시 방어선. 마스터 결정(2026-04-30).
- 초과 시 즉시 raise + 남은 limit 표시.

감사 로그:
- 모든 outbound는 `audit_log` 테이블에 기록.
- 기록 필드: timestamp KST, action, format, channel, session_id, payload_size, status, masked_count.

---

### 8.2 Inbound 보안

HMAC 서명 검증: 6.2절 절차 적용.

시간 윈도우: timestamp ±5분 이내만 허용. 외부 NTP 동기화 권장.

화이트리스트: IP + 도메인 + secp256k1 주소 기반 삼중 확인. 빈 목록이 기본(차단 우선).

페이로드 크기 상한: 1 MB. 초과 시 즉시 413 reject.

검증 실패 처리: 즉시 reject (해당 HTTP 에러 코드) + `transfer_logs`에 `status=rejected` 기록 + 연속 5회 실패 시 마스터 Telegram 알림 + 해당 발신자 임시 차단 (1시간).

---

### 8.3 절대 규칙 (마스터 메모리 반영)

- fallback 금지: 모든 검증 실패, 전송 실패, 파싱 실패는 즉시 raise. 조용히 넘어가지 않는다.
- 외부 발송 자동 승인 금지: 명시 승인(confirm 또는 mfa)만 허용. 발송 이력 없는 새 대상은 항상 confirm.
- 개인 데이터 유출 금지: 태그 제외(1차) + 키 패턴 마스킹(2차) 이중 방어.
- 롤백 가능 후 자동 승인: 모든 outbound는 audit_log에 기록 → 추후 추적·취소 가능한 상태.
- 되돌릴 수 없는 작업(외부 발송, webhook 등록): 마스터 confirm 필수.
- 시간대 KST: 모든 timestamp는 `Asia/Seoul` 기준. UTC 사용 금지.

---

## 9. 데이터 모델 (DB 스키마)

SQLite 기반. 파일: `~/.openxgram/store.db` (기존 사이드카 DB에 추가 테이블).

DB 드라이버: rusqlite (sync). sqlx는 sqlite-vec 통합이 번거로워 제외. (마스터 결정 2026-04-30)
rusqlite silent error 주의: `execute()` 후 `affected_rows()` 검증 필수. UPDATE/DELETE 0건은 통상 버그로 취급하여 즉시 raise.

### 테이블: transfer_logs

```sql
CREATE TABLE transfer_logs (
  id            TEXT PRIMARY KEY,           -- UUID v4
  direction     TEXT NOT NULL,              -- 'outbound' | 'inbound'
  channel       TEXT NOT NULL,              -- 'clipboard' | 'email' | 'telegram' | 'discord' | 'webhook' | 'file'
  format        TEXT NOT NULL,              -- 'text-package' | 'file' | 'code' | 'webhook'
  session_id    TEXT,                       -- 연관 세션 ID
  payload_size  INTEGER,                    -- 바이트 단위
  status        TEXT NOT NULL,              -- 'success' | 'failed' | 'rejected' | 'conflict-skipped'
  sender        TEXT,                       -- secp256k1 주소 (inbound 발신자 또는 outbound 자기 주소)
  receiver      TEXT,                       -- 대상 주소 또는 URL
  signature     TEXT,                       -- 서명 hex (있는 경우)
  masked_count  INTEGER DEFAULT 0,          -- 마스킹된 항목 수
  error_message TEXT,                       -- 오류 발생 시 메시지
  timestamp     TEXT NOT NULL               -- ISO 8601 KST
);
```

### 테이블: webhook_endpoints

```sql
CREATE TABLE webhook_endpoints (
  id            TEXT PRIMARY KEY,           -- UUID v4
  name          TEXT NOT NULL UNIQUE,       -- 사람이 읽을 수 있는 이름
  transport     TEXT NOT NULL,              -- 'http-post' | 'discord-webhook'
  target_url    TEXT NOT NULL,              -- 대상 URL
  auth_type     TEXT NOT NULL,              -- 'hmac' | 'none'
  secret_vault_key TEXT,                   -- vault 키 이름 (HMAC 시크릿)
  triggers      TEXT NOT NULL,             -- JSON 배열: ["pin", "session-end", "schedule", "merge"]
  cron_expr     TEXT,                       -- cron 표현식 (schedule 트리거 시)
  rate_limit    INTEGER DEFAULT 10,         -- 시간당 최대 호출 수
  created_at    TEXT NOT NULL,              -- ISO 8601 KST
  last_used_at  TEXT                        -- ISO 8601 KST
);
```

### 테이블: webhook_acl

```sql
CREATE TABLE webhook_acl (
  id             TEXT PRIMARY KEY,          -- UUID v4
  endpoint_id    TEXT NOT NULL,             -- webhook_endpoints.id 참조
  allowed_type   TEXT NOT NULL,             -- 'address' | 'ip' | 'domain'
  allowed_value  TEXT NOT NULL,             -- 주소/IP/도메인 값
  created_at     TEXT NOT NULL,             -- ISO 8601 KST
  FOREIGN KEY (endpoint_id) REFERENCES webhook_endpoints(id) ON DELETE CASCADE
);
```

### 테이블: audit_log

```sql
CREATE TABLE audit_log (
  id            TEXT PRIMARY KEY,           -- UUID v4
  transfer_id   TEXT NOT NULL,              -- transfer_logs.id 참조
  action        TEXT NOT NULL,              -- 'extract' | 'import' | 'webhook-trigger' | 'masked'
  actor         TEXT NOT NULL,              -- 'master' | 'system' | secp256k1 주소
  detail        TEXT,                       -- JSON 상세 정보
  timestamp     TEXT NOT NULL               -- ISO 8601 KST
);
```

---

## 10. 자동 트리거

### on session-end

발화 시점: `xgram session end` 또는 데몬이 비활성 감지 후 세션 자동 종료 시.

페이로드: 종료된 세션의 L0 메시지 전체 + L2 memories.

대상: 등록된 webhook_endpoints 중 `triggers` 배열에 `session-end` 포함된 것 전체.

권장 설정: 이메일 또는 Discord 백업.

---

### on pin

발화 시점: 마스터가 `xgram memory pin <id>` 또는 TUI/GUI에서 핀 버튼 누를 때.

페이로드: 핀된 L2 memory 항목 단건.

대상: `triggers` 배열에 `pin` 포함된 webhook_endpoints 전체.

권장 설정: Notion, Linear 등 외부 시스템 webhook.

---

### on schedule

발화 시점: `cron_expr` 에 따라 시스템 cron 또는 내장 스케줄러가 발화.

페이로드: `since` 파라미터에 따라 기간 내 L2 memories + 핀된 항목.

대상: `triggers` 배열에 `schedule` 포함된 webhook_endpoints 전체.

권장 설정: Discord 일일 백업 `"0 18 * * *"`.

---

### on memory-merge

발화 시점: L3 패턴 클러스터링에서 기존 메모리가 병합될 때 (Phase 2).

페이로드: 병합된 메모리 묶음.

대상: `triggers` 배열에 `merge` 포함된 webhook_endpoints 전체.

권장 설정: Phase 2 이후 적용.

---

## 11. 에러 처리 (fallback 금지 적용)

모든 에러는 raise다. 조용히 무시하는 경로는 없다.

Silent Error 4패턴 필수 적용:
- reqwest: 모든 HTTP 호출에 `.error_for_status()?` 강제. XMTP REST 포함 예외 없음.
- rusqlite: `execute()` 후 `affected_rows()` 검증. 0건 → raise.
- tokio-cron-scheduler: job panic 핸들러 + tracing 로깅 필수. silent 흡수 금지.
- keyring: 저장 후 round-trip get 검증. headless Linux silent 실패 방지.

네트워크 실패:
- 연결 거부, 타임아웃 → 즉시 오류 표시.
- 5xx 응답 → 명시 retry (최대 3회, 지수 백오프 30s/60s/120s).
- 3회 모두 실패 → raise + 마스터 Telegram 알림.
- silent retry 금지: 모든 retry는 로그에 기록.

서명 검증 실패:
- 즉시 raise 및 HTTP 401 응답.
- `transfer_logs` 에 `status=rejected, error_message=SIGNATURE_INVALID` 기록.
- 연속 5회 실패 → 해당 주소 임시 차단 1시간 + 마스터 알림.

페이로드 파싱 실패:
- 즉시 raise 및 HTTP 400 응답.
- 원본 페이로드를 `~/.openxgram/failed/` 디렉토리에 타임스탬프 파일로 보관. 자동 삭제 없음.
- 마스터가 `xgram debug import --file ~/.openxgram/failed/<FILE>` 로 재처리 가능.

디스크 가득 참:
- 즉시 raise + 마스터 Telegram 알림.
- 남은 용량이 100 MB 이하: 경고만.
- 10 MB 이하: 모든 outbound 차단 + raise.
- 조용한 무시 금지.

임베딩 실패 (multilingual-e5-small):
- 즉시 raise. `--embed defer` 모드가 아닌 한.
- `--embed defer` 모드: 실패 항목을 `embedding_queue` 테이블에 추가. 야간 reflection에서 재시도.
- 3회 연속 실패: raise + 로그.

---

## 12. 시나리오 예시 5개 (PRD 18절 확장)

### 시나리오 A — ChatGPT 웹과 토론 후 사이드카로 import

상황: 마스터가 ChatGPT 웹 브라우저 창에서 OpenXgram 아키텍처를 논의하고 결론을 얻었다. 이 대화 내용을 사이드카로 가져와야 한다.

Step-by-step:
1. 마스터가 `xgram extract --format text-package --target clipboard --pinned-only` 실행.
2. 사이드카가 현재 세션의 핀된 항목을 Text Package로 생성해 클립보드에 복사.
3. 마스터가 ChatGPT 창에 붙여넣기 → "이 맥락을 바탕으로 MT 모듈 결정사항을 추가해줘" 입력.
4. ChatGPT 응답 전체를 마스터가 클립보드에 복사.
5. `xgram session import --from clipboard --new` 실행.
6. MT 모듈이 형식 자동 감지 (plain text → text-package 감지 시도, 실패 시 markdown으로 파싱).
7. L0 메시지로 삽입, `decision` 패턴 포함 줄은 L2 memory 후보로 표시.
8. BGE-small 임베딩 즉시 생성.

예상 결과: 새 세션에 ChatGPT 대화 내용이 들어오고 회상 검색에서 `xgram.search("MT 모듈")` 으로 바로 찾을 수 있다.

검증 포인트:
- `xgram session list` 에서 새 세션 확인.
- `xgram memory search "MT 모듈"` 에서 관련 항목 상위 노출 확인.
- 임베딩 생성 여부 확인 (`xgram memory info <id>` 에서 `embedding: true`).

---

### 시나리오 B — 사이드카 → Notion에 결정 자동 export (webhook)

상황: 마스터가 pin할 때마다 Notion 페이지가 자동으로 업데이트되길 원한다.

Step-by-step:
1. `xgram webhook add --name notion-decisions --transport http-post --target https://hook.notion.so/... --trigger pin` 등록.
2. Notion 측에서 xgram 공개 주소를 webhook_acl에 추가.
3. 마스터가 TUI에서 L2 memory 항목에 핀 설정.
4. `on-pin` 트리거 발화 → 사이드카가 해당 항목 Webhook Payload 생성.
5. `X-Xgram-Signature` 헤더 생성 후 Notion hook URL로 POST.
6. Notion 측에서 HMAC 검증 → 페이지 자동 업데이트.
7. 응답 200 → `transfer_logs` 에 `status=success` 기록.

예상 결과: 마스터가 pin 누른 후 5초 이내 Notion 페이지에 항목이 추가된다.

검증 포인트:
- `xgram webhook test --name notion-decisions` 로 테스트 payload 발송 후 Notion 확인.
- `transfer_logs` 에서 status=success 항목 확인.
- Notion 페이지에서 내용 + 타임스탬프 일치 확인.

---

### 시나리오 C — Cloudflare 설정을 코드 형태로 추출

상황: 마스터가 과거에 결정한 Cloudflare DNS 설정들을 Python 스크립트로 재현하고 싶다.

Step-by-step:
1. `xgram extract --format code --lang python --tag cloudflare --preview` 실행.
2. MT 모듈이 `cloudflare` 태그 L2 memories와 vault 항목 검색.
3. `--preview` 이므로 발송 없이 stdout에 수집된 항목 목록 출력.
4. 마스터가 목록 확인 후 `xgram extract --format code --lang python --tag cloudflare --target clipboard` 실행.
5. 승인 정책 `confirm` → TUI에서 마스터 확인.
6. 수집 항목을 Gemini API에 전달 (변환 프롬프트: "아래 결정사항을 Python Cloudflare API 스크립트로 변환하라. API 키는 [REDACTED]로 대체하라").
7. 반환된 Python 코드 블록을 클립보드에 복사.
8. `audit_log` 에 기록.

예상 결과: 마스터가 클립보드에서 Python 스크립트를 붙여넣으면 API 키만 교체해 바로 실행 가능한 상태.

검증 포인트:
- 코드 내 API 키가 `[REDACTED]` 로 치환됐는지 확인.
- Python 스크립트 문법 오류 없는지 `python3 -m py_compile` 로 확인.
- `audit_log` 에서 `format=code, lang=python` 항목 확인.

---

### 시나리오 D — 외부 시스템(Linear)이 사이드카에 작업 결과 inbound

상황: Linear에서 이슈가 완료될 때마다 사이드카가 자동으로 기억하도록 설정한다.

Step-by-step:
1. `xgram webhook add --name linear-inbound --transport http-post --target http://localhost:14920/xgram/inbound` 등록.
2. `xgram webhook acl add --name linear-inbound --domain linear.app` 화이트리스트 추가.
3. Linear 설정에서 완료 이벤트 webhook을 `http://GCP_IP:14920/xgram/inbound` 로 설정.
4. Linear에서 이슈 완료 → webhook 발화.
5. 사이드카 inbound endpoint 수신 → timestamp 확인 → 도메인 화이트리스트 확인 → HMAC 검증.
6. 페이로드 파싱 → L0 메시지로 삽입, `issue-complete` 태그 L2 memory 생성.
7. 응답 `{"status":"ok","session_id":"...","messages_inserted":1,"memories_inserted":1}`.

예상 결과: `xgram memory search "Linear 이슈"` 로 완료된 이슈 목록을 회상할 수 있다.

검증 포인트:
- `xgram webhook test --name linear-inbound` 로 수동 테스트.
- `transfer_logs` 에서 `direction=inbound, status=success` 확인.
- `xgram memory search "issue-complete"` 에서 항목 검색 확인.

---

### 시나리오 E — Discord에 매일 18시 자동 백업

상황: 마스터가 Discord #xgram-backup 채널에서 매일 저녁 당일 결정 사항을 확인하고 싶다.

Step-by-step:
1. `xgram backup-push --channel discord --cron "0 18 * * *"` 등록.
2. 내부적으로 `xgram webhook add --name daily-discord-backup --transport discord-webhook --target WEBHOOK_URL --trigger schedule --cron "0 18 * * *"` 등록.
3. 매일 18시 KST 스케줄러 발화.
4. 당일 L2 memories와 핀된 항목을 Markdown 파일로 추출.
5. Discord webhook으로 파일 첨부 전송. 메시지: `[OpenXgram 일일 백업] 2026-04-30`.
6. 4000자 초과 시 파일 첨부로 자동 전환 (이미 파일 첨부 방식 기본 사용).
7. `transfer_logs` 에 기록.

예상 결과: 매일 18시에 Discord #xgram-backup 채널에 당일 결정 사항 마크다운 파일이 올라온다.

검증 포인트:
- `xgram webhook test --name daily-discord-backup` 로 수동 발화.
- Discord 채널에서 파일 첨부 메시지 확인.
- `transfer_logs` 에서 `channel=discord, status=success` 확인.

---

## 13. 테스트 계획 (Qua용)

### 13.1 단위 테스트 케이스

- `test_extract_text_package_basic` — 메시지 50개 세션에서 Text Package 추출, 구조 검증.
- `test_extract_secret_tag_excluded` — `secret` 태그 항목이 payload에 없는지 확인.
- `test_masking_api_key_pattern` — hex 토큰 패턴 자동 마스킹 적용 여부.
- `test_masking_bip39_seed` — 12단어 BIP39 시드 패턴 탐지 및 마스킹.
- `test_webhook_signature_generation` — secp256k1 서명 생성 후 검증 일치.
- `test_webhook_signature_mismatch_raises` — 서명 불일치 시 raise 확인.
- `test_timestamp_window_expired_raises` — 5분 초과 timestamp 즉시 reject 확인.
- `test_inbound_parse_markdown` — Markdown 형식 inbound 자동 감지 및 L0 삽입.
- `test_inbound_parse_json` — JSON 형식 inbound 파싱 및 L2 삽입.
- `test_duplicate_message_skip` — 동일 서명 메시지 재삽입 시 skip (오류 아님) 확인.
- `test_rate_limit_raises` — 시간당 10회 초과 시 raise 확인.
- `test_embed_failure_raises` — BGE-small 실패 시 raise (auto 모드).
- `test_payload_too_large_raises` — 1 MB 초과 페이로드 즉시 reject 확인.
- `test_disk_full_raises` — 디스크 10 MB 이하 시 outbound 차단 확인.

---

### 13.2 통합 시나리오

- `integration_clipboard_roundtrip` — extract → clipboard → import 완전 왕복. 메시지 수, 기억 수 일치 확인.
- `integration_webhook_pin_trigger` — pin 이벤트 → webhook outbound → mock 수신 서버 응답 200 → transfer_logs 기록 확인.
- `integration_inbound_session_new` — mock 외부 서버가 inbound endpoint로 POST → 새 세션 생성 확인.
- `integration_schedule_cron` — cron 5분 주기로 짧게 설정 후 자동 발화 확인.
- `integration_code_extraction_python` — `cloudflare` 태그 항목 Python 코드 변환 (Gemini API mock 사용).

---

### 13.3 보안 케이스

- `security_malformed_signature` — 잘못된 서명 형식 inbound → 즉시 400 + transfer_logs rejected 확인.
- `security_replay_attack` — 6분 전 timestamp 재사용 → 즉시 401 확인.
- `security_oversized_payload` — 1.1 MB payload → 즉시 413 확인.
- `security_whitelist_denied` — 화이트리스트 미등록 주소 inbound → 즉시 403 + 마스터 알림 확인.
- `security_secret_tag_in_outbound` — `secret` 태그 항목이 extract payload에 포함되지 않는지 확인.
- `security_five_consecutive_failures_block` — 연속 5회 서명 실패 후 해당 주소 1시간 차단 확인.

---

### 13.4 성능 케이스

- `perf_10mb_payload_inbound` — 10 MB payload (1 MB 상한으로 즉시 reject 확인).
- `perf_large_session_extract` — 5 MB 세션 Text Package 추출 시간 5초 이내 확인.
- `perf_100_concurrent_outbound` — 100개 동시 outbound 요청 시 rate limit 작동 및 응답 일관성 확인.
- `perf_embedding_100_messages` — 100개 메시지 임베딩 일괄 생성 30초 이내 완료 확인 (BGE-small).

---

## 14. Phase 분할

### Phase 1 MVP (5~6일)

구현 항목:
- Push: Text Package 추출 + Markdown 파일 추출
- 대상 채널: 클립보드 + Discord (webhook 방식)
- Pull: 클립보드 붙여넣기 import
- CLI: `xgram extract`, `xgram backup-push`, `xgram session import` 기본 명령
- TUI: Memory Transfer 기본 화면 (Push/Pull 탭)
- audit_log + transfer_logs 테이블 생성 + 기록
- 태그 제외 + 키 패턴 마스킹

---

### Phase 1.5

구현 항목:
- Email outbound (SMTP)
- Telegram outbound (@starianbot)
- Webhook outbound 기본 (http-post, HMAC 서명)
- 코드 추출 (Python, SQL만, Gemini API 연동)
- TUI: webhook 관리 탭 추가

---

### Phase 2

구현 항목:
- Inbound webhook (HMAC 검증 + 화이트리스트 + 스키마 검증)
- GUI Memory Transfer 페이지 (Tauri + React)
- 모든 추출 형식 완성 (File .json/.yaml, Code TypeScript/Nginx/Bash/JSON config)
- 드래그앤드롭 파일 import (GUI)
- 자동 트리거 전체 (on-pin, on-session-end, on-schedule)

---

### Phase 2+

구현 항목:
- 브라우저 확장에서 웹 LLM 대화 자동 감지 + import
- 클립보드 데몬: 마스터가 클립보드에 복사하면 자동 import 감지
- Telegram inbound (Phase 1.5 미구현 분)
- on-memory-merge 트리거 (L3 패턴 연동)

---

## 15. 의존성

### 사이드카 내부 모듈 인터페이스

- keystore 모듈: secp256k1 서명 생성/검증, 공개 주소 조회. MT 모듈이 서명 시 keystore에 위임.
- memory engine: 임베딩 검색(search), 메시지/에피소드/메모리 CRUD. outbound 추출과 inbound 삽입 모두 memory engine API 경유.
- share_policy: 자동 트리거(on-pin, on-session-end 등) 이벤트 수신. MT 모듈이 share_policy 이벤트를 구독.
- vault: `secret`/`vault` 태그 항목 자동 제외 판정. SMTP 비밀번호, 봇 토큰 등 민감 설정 조회.

### 외부 라이브러리

이메일:
- `lettre` (Rust) — SMTP STARTTLS 클라이언트.

HTTP 클라이언트:
- `reqwest` (Rust) — outbound webhook, Discord webhook, Telegram Bot API 호출.

HTTP 서버 (inbound endpoint):
- `axum` (Rust) — 사이드카 HTTP 서버. 기존 MCP 서버와 동일 프레임워크 사용 검토.

암호화:
- `secp256k1` (Rust) — 서명 생성/검증. 기존 keystore 모듈과 공유.

Discord:
- Discord REST API 직접 호출 (`reqwest` 경유). 별도 SDK 없이 처리.

Telegram:
- Telegram Bot API 직접 호출 (`reqwest` 경유). 별도 SDK 없이 처리.

스케줄러:
- `tokio-cron-scheduler` (Rust) — on-schedule 트리거.

---

## 16. 결정 필요 항목 — 전체 확정 (2026-04-30)

모든 항목 마스터 결정 완료. 미결 사항 없음.

- 이메일 어댑터: **영구 제외**. 백업 채널은 클립보드·Telegram·Discord 3종으로 확정.
- Discord 백업 전용 채널 이름: `#xgram-backup` 제안 유지. 마스터가 채널 생성 후 ID 등록.
- Discord 서버 ID와 채널 ID: 마스터가 `xgram config set` 명령으로 직접 등록.
- Webhook 화이트리스트 초기 도메인: **빈 목록**으로 시작. 마스터가 use 시점에 `xgram webhook acl add`로 추가.
- 코드 추출 LLM: **사용자 선택 어댑터 패턴** 확정 (Gemini / Claude / OpenAI / Ollama / Template). 4.3.1절 참조.
- Inbound webhook HTTP 포트: **14921** 확정. 14920은 다른 용도 예약.
- Rate limit: **분당 1000건**. 안전망. 정상 사용 영향 없음. fallback 아닌 명시 방어선.
- on-schedule 기본 cron: **기본 미설정**. 온보딩 Step 7에서 마스터가 직접 입력.
