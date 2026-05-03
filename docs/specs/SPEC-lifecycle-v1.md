# SPEC — Lifecycle Manager Module v1

작성일: 2026-04-30 (KST)
버전: v0.1.0.0-alpha.1
상태: 초안
작성자: Pip (agt_dept_prd)
기반: 마스터 누적 결정 (fallback 금지, 롤백 후 자동 승인, trash 우선, KST 타임스탬프)

---

## 1. 개요

### 1.1 한 줄 정의

"사이드카의 설치·제거·리셋·마이그레이션·점검을 책임지는 모듈"

### 1.2 본질

Lifecycle Manager는 OpenXgram 사이드카의 탄생부터 소멸, 그리고 재탄생까지 전 생애주기를 추적하고 보장하는 단일 진입점이다.
마스터가 테스트 목적으로 install → 사용 → uninstall을 반복할 때 시스템 어디에도 흔적이 남지 않아야 한다는 원칙이 이 모듈의 존재 이유다.
모든 검증 실패는 즉시 오류를 raise하고 마스터에게 알린다. silent 무시는 절대 금지다.

### 1.3 핵심 원칙 5가지

- fallback 금지: 검증 실패 시 조용히 넘어가지 않는다. 즉시 오류를 raise하고 마스터에게 알린다.
- 롤백 가능 후 자동 승인: 백업 검증이 통과된 상태에서만 제거 단계를 진행한다.
- 삭제 전 정체 확인 필수: manifest 무결성과 시드 서명을 검증한 후에만 파일을 건드린다.
- trash 우선: `rm -rf` 대신 trash로 이동하여 7일간 복구 가능 상태를 보장한다.
- KST 타임스탬프: 모든 로그, manifest, 백업 파일명에 KST 기준 ISO8601을 사용한다.

---

## 2. 용어

### 2.1 워크플로우 단위

- Onboarding: 사이드카를 처음 설치하는 전체 흐름. `xgram init`으로 시작한다.
- Installation: Onboarding 중 파일·서비스·DB를 실제로 배치하는 단계.
- Uninstallation: 설치된 모든 구성요소를 역순으로 제거하는 흐름. `xgram uninstall`로 시작한다.
- Reset: 사이드카를 특정 기준점 상태로 되돌리는 흐름. 키·설정·데이터를 선택적으로 유지한다.
- Migration: 버전 업그레이드 시 DB 스키마와 설정을 새 버전에 맞게 변환하는 흐름.
- Doctor: 현재 설치 상태의 건강을 점검하는 비파괴적 진단 흐름.

### 2.2 핵심 개념

- Manifest: install-manifest.json. 설치된 모든 파일, 서비스, 키, 포트를 기록하는 source-of-truth 문서.
- uninstall_token: 마스터 시드로 서명한 제거 권한 토큰. manifest에 포함되며 위변조 방지에 사용된다.
- managed 외부 리소스: Lifecycle Manager가 생성하고 등록한 리소스. Discord webhook, Telegram bot 등. 제거 시 정리 대상이다.
- unmanaged 외부 리소스: 마스터가 직접 만들었거나 외부 시스템이 소유한 리소스. 제거 시 절대 건드리지 않는다.
- Idempotent: 같은 명령을 여러 번 실행해도 결과가 동일하다. `xgram uninstall`을 2회 연속 실행해도 안전하다.
- Dry-run: 실제 파일 시스템 변경 없이 수행될 작업 목록만 출력한다.
- Drift detection: manifest에 기록된 상태와 실제 파일 시스템 상태의 차이를 감지한다.

---

## 3. 온보딩 9단계 (xgram init 워크플로우)

### 개요

`xgram init` 실행 시 9단계 대화형 마법사가 순서대로 진행된다.
각 단계는 독립적으로 검증되며, 실패 시 해당 단계에서 즉시 오류를 raise한다. 이전 단계로 돌아가거나 skip하여 넘어가는 fallback은 허용하지 않는다.
진행률은 `[3/9] 마스터 시드` 형식으로 표시한다.

---

### Step 1 — 환영 + 사전 점검

목적: 설치를 진행하기 위한 최소 환경을 확인한다.

사용자 입력 항목:
- 없음 (자동 점검)

자동 검증 규칙:
- 디스크 여유 공간 ≥ 500MB
- 필수 포트(기본 7300, 7301) 미점유 확인
- 실행 권한 (홈 디렉토리 쓰기 가능)
- OS 감지 (linux/macos/windows)
- 기존 설치 여부 확인 (install-manifest.json 존재 시 중단 또는 --force 플래그 요구)

실패 시 처리:
- 디스크 부족: 오류 raise, 부족량 명시, 종료
- 포트 점유: 점유 프로세스 PID와 이름을 출력하고 오류 raise
- 기존 설치 감지: "이미 설치되어 있습니다. `xgram uninstall` 후 재시도하거나 `xgram init --force`를 사용하세요." 출력 후 종료

진행률 표시: `[1/9] 사전 점검`

---

### Step 2 — 머신 식별

목적: 이 사이드카 인스턴스의 정체성을 설정한다.

사용자 입력 항목:
- 머신 별칭 (예: gcp-main, macmini-home). 기본값: 호스트명
- 머신 역할 (primary/secondary/worker). 기본값: primary
- Tailscale IP (자동 감지 시 확인, 없으면 스킵)

자동 검증 규칙:
- 별칭: 영숫자와 하이픈만 허용, 1~32자
- Tailscale: `tailscale status` 실행 가능 시 자동 감지, IP 형식 검증 (100.x.x.x)
- 같은 별칭을 가진 피어가 네트워크에 이미 있으면 WARN 출력 (오류는 아님)

실패 시 처리:
- 별칭 형식 오류: 재입력 요청

진행률 표시: `[2/9] 머신 식별`

---

### Step 3 — 마스터 시드

목적: 모든 키 파생과 서명의 근원이 되는 마스터 시드를 설정한다.

사용자 입력 항목:
- 신규 생성 또는 기존 시드 import 선택
- 신규: 24단어 BIP39 니모닉 화면에 표시 → 마스터가 기록 후 Y/N 확인 입력
  - Y: 다음 단계 진행
  - N: "시드 백업 없이는 진행할 수 없습니다." 후 종료
  - 재입력 옵션(권장, 강제 아님): 마스터가 원하면 24단어를 직접 재입력하여 검증 (`xgram init --verify-seed`)
- import: 24단어 직접 입력

마스터 결정(2026-04-30): 24단어 표시 + Y/N 확인 + 재입력 옵션 권장. 재입력 강제 아님.

자동 검증 규칙:
- BIP39 단어 목록 검증 (2048개 단어셋)
- 체크섬 검증
- 신규 생성 시: 엔트로피 소스 `/dev/urandom` 또는 OS CSPRNG
- import 시: 단어 수 정확히 24개

실패 시 처리:
- 체크섬 불일치: "시드가 유효하지 않습니다. 단어를 다시 확인하고 재입력하세요." 후 재입력 요청
- 마스터가 N을 입력 시 (백업 확인 거부): "시드 백업 없이는 진행할 수 없습니다." 후 종료

진행률 표시: `[3/9] 마스터 시드`

---

### Step 4 — 마스터 키페어

목적: 시드에서 HD(Hierarchical Deterministic) 키페어를 파생하고 암호화 저장한다.

사용자 입력 항목:
- keystore 패스워드 (입력 + 확인 입력, 화면 미표시)

자동 검증 규칙:
- 패스워드 최소 12자
- HD 파생: BIP44 경로 `m/44'/60'/0'/0/0` (이더리움 호환)
- scrypt KDF: N=2^17, r=8, p=1 (기본), 메모리 제약 환경에서 N=2^14
- keystore 저장 위치: `~/.openxgram/keystore/master.json` (600 권한)

실패 시 처리:
- 패스워드 길이 미달: 재입력 요청
- 패스워드 불일치: 재입력 요청
- keystore 파일 쓰기 실패: 오류 raise, 권한 문제 명시

진행률 표시: `[4/9] 마스터 키페어`

---

### Step 5 — 데이터 디렉토리

목적: 모든 영구 데이터가 저장될 루트 디렉토리를 확정한다.

사용자 입력 항목:
- 데이터 디렉토리 경로. 기본값: **`~/.openxgram/`** (마스터 결정 2026-04-30)

자동 검증 규칙:
- 경로 생성 가능 여부 (쓰기 권한)
- 디스크 여유 공간 ≥ Step 1에서 확인한 값 재검증
- 기존 데이터 있으면 WARN 출력 ("기존 데이터가 있습니다. 덮어쓰면 손실됩니다.")
- 심볼릭 링크 허용 (실제 경로로 resolve하여 기록)

실패 시 처리:
- 쓰기 권한 없음: 오류 raise, 권한 명시

진행률 표시: `[5/9] 데이터 디렉토리`

---

### Step 6 — 데이터베이스 초기화

목적: SQLite DB를 초기화하고 임베딩 모델을 준비한다.

DB 드라이버: rusqlite (sync). sqlx는 sqlite-vec 통합이 번거로워 제외. rusqlite `execute()` 후 `affected_rows()` 검증 필수. (마스터 결정 2026-04-30)

사용자 입력 항목:
- 없음 (자동)

자동 검증 규칙:
- SQLite 버전 ≥ 3.40.0
- sqlite-vec 확장 로드 가능 여부 확인
- 초기 마이그레이션 SQL 파일 순차 적용 (파일명 순서: 001_initial.sql, 002_vec.sql ...)
- multilingual-e5-small 모델 다운로드 (캐시: `~/.openxgram/models/`, 560MB, fastembed 통합, 한국어/영어 지원. 마스터 결정 2026-04-30)
- 모델 SHA256 해시 검증. hash 불일치 시 다운로드 파일 삭제 후 즉시 raise. silent 실패 금지.

실패 시 처리:
- SQLite 버전 미달: 오류 raise, 필요 버전 명시
- sqlite-vec 로드 실패: 오류 raise, 빌드 의존성 안내
- 마이그레이션 SQL 실패: 오류 raise, 실패한 SQL 파일명과 오류 내용 명시
- 모델 해시 불일치: 다운로드 파일 삭제 후 오류 raise

진행률 표시: `[6/9] 데이터베이스 초기화`

---

### Step 7 — 외부 어댑터

목적: Discord, Telegram, SMTP, OpenAgentX 등 외부 서비스 연동을 설정한다.

사용자 입력 항목:
- 각 어댑터별 활성화 여부 (Y/N, 기본 N — 모두 스킵 가능)
- Discord: Bot Token, Channel ID
- Telegram: Bot Token, Chat ID
- OpenAgentX: API 키, 엔드포인트 URL
- 자동 백업 스케줄 (옵션): cron 표현식 입력. 예: `0 18 * * *` (매일 18시 KST). 기본 미설정. 마스터 결정(2026-04-30).
- 코드 추출 LLM 어댑터 선택 (옵션): Gemini / Claude / OpenAI / Ollama / Template 중 선택. 기본 `gemini`. 마스터 결정(2026-04-30).

이메일(SMTP) 어댑터는 영구 제외. 마스터 결정(2026-04-30).

자동 검증 규칙:
- 활성화된 어댑터만 검증
- Discord: 토큰 형식 확인 + `GET /users/@me` API 호출로 유효성 검증 (만료 여부)
- Telegram: `getMe` API 호출로 검증 (토큰 유효성만, 실제 메시지 전송 금지)
- SMTP: TLS 연결 및 EHLO 응답 확인
- OpenAgentX: `/health` 엔드포인트 응답 확인

실패 시 처리:
- 토큰 만료 또는 잘못된 토큰: "어댑터 검증 실패: [어댑터명] - [오류 내용]" 출력 후 재입력 요청 또는 스킵 선택
- 모든 어댑터는 스킵 가능. 단, 스킵 시 manifest에 `enabled: false`로 기록

진행률 표시: `[7/9] 외부 어댑터`

---

### Step 8 — Transport

목적: 사이드카 간 통신 방식을 선택한다.

사용자 입력 항목:
- Transport 방식 선택: localhost / Tailscale / XMTP
- localhost: 포트 번호 (기본 7300)
- Tailscale: 이미 감지된 IP 사용 또는 수동 입력
- XMTP: 클라이언트 주소 (Step 4 키페어에서 자동 파생)

자동 검증 규칙:
- localhost: 포트 미점유 확인 (Step 1과 동일)
- Tailscale: `tailscale ping {self_ip}` 응답 확인
- XMTP: 클라이언트 초기화 및 네트워크 연결 확인

실패 시 처리:
- Tailscale 연결 불가: 오류 raise, "Tailscale이 실행 중인지 확인하세요."
- XMTP 연결 불가: 오류 raise, 네트워크 상태 및 XMTP 네트워크 상태 안내

진행률 표시: `[8/9] Transport`

---

### Step 9 — 데몬 등록

목적: 사이드카 데몬을 OS 서비스로 등록하여 재부팅 후 자동 시작을 보장한다.

사용자 입력 항목:
- 서비스 자동 등록 여부: 대화형 `[Y/n]` 확인. 기본 **Y** (엔터 입력 시 자동 등록). 마스터 결정(2026-04-30).
- user-level vs system-level 선택 (Linux: systemd user vs system, macOS: user LaunchAgent vs system LaunchDaemon)

자동 검증 규칙:
- OS 감지에 따라 적절한 서비스 관리자 선택 (§11 참조)
- unit file / plist / NSSM 서비스 파일 생성
- 서비스 등록 후 즉시 시작 시도
- 시작 후 5초 이내 healthcheck 응답 확인 (`GET /health`)

실패 시 처리:
- 서비스 등록 실패: 오류 raise, 수동 등록 명령어 출력
- healthcheck 실패: 서비스 중단 + 서비스 파일 삭제 + 오류 raise + 로그 경로 안내

진행률 표시: `[9/9] 데몬 등록`

---

### 비대화 모드

`xgram init --config FILE`: TOML 또는 JSON 설정 파일로 9단계를 비대화 방식으로 진행한다. CI/자동화 환경에서 사용한다. 시드는 환경변수 `XGRAM_SEED`로 주입한다 (파일에 평문 저장 금지).

`xgram init --import SEED`: 다른 머신에서 동일 시드로 추가 인스턴스를 설치한다. Step 3에서 시드 입력을 스킵하고 `--import` 인자를 사용한다.

`xgram init --dry-run`: 실제 파일 시스템 변경 없이 수행될 작업 목록만 출력한다. manifest 예상 내용도 함께 출력한다.

---

## 4. install-manifest.json 스키마

### 4.1 JSON Schema

```
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "type": "object",
  "required": ["version", "installed_at", "machine", "uninstall_token", "files",
               "directories", "system_services", "binaries", "shell_integrations",
               "external_resources", "registered_keys", "ports", "os_keychain_entries"],
  "properties": {
    "version": {
      "type": "string",
      "description": "manifest 스키마 버전 (OpenXgram 버전 아님)",
      "example": "1"
    },
    "installed_at": {
      "type": "string",
      "format": "date-time",
      "description": "설치 완료 시각. KST ISO8601 형식. 예: 2026-04-30T14:32:00+09:00"
    },
    "machine": {
      "type": "object",
      "required": ["alias", "role", "os", "arch", "hostname"],
      "properties": {
        "alias":     { "type": "string" },
        "role":      { "type": "string", "enum": ["primary", "secondary", "worker"] },
        "os":        { "type": "string", "enum": ["linux", "macos", "windows"] },
        "arch":      { "type": "string" },
        "hostname":  { "type": "string" },
        "tailscale_ip": { "type": ["string", "null"] }
      }
    },
    "uninstall_token": {
      "type": "string",
      "description": "마스터 시드로 서명한 제거 권한 토큰. base64url 인코딩된 EdDSA 서명."
    },
    "files": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["path", "sha256", "size_bytes", "installed_at"],
        "properties": {
          "path":         { "type": "string" },
          "sha256":       { "type": "string" },
          "size_bytes":   { "type": "integer" },
          "installed_at": { "type": "string", "format": "date-time" }
        }
      }
    },
    "directories": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["path", "created_by_installer"],
        "properties": {
          "path":               { "type": "string" },
          "created_by_installer": { "type": "boolean" }
        }
      }
    },
    "system_services": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["name", "type", "unit_file", "enabled", "started"],
        "properties": {
          "name":       { "type": "string" },
          "type":       { "type": "string", "enum": ["systemd-user", "systemd-system", "launchd-user", "launchd-system", "windows-service"] },
          "unit_file":  { "type": "string", "description": "서비스 파일의 절대 경로" },
          "enabled":    { "type": "boolean" },
          "started":    { "type": "boolean" }
        }
      }
    },
    "binaries": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["path", "sha256", "version"],
        "properties": {
          "path":    { "type": "string" },
          "sha256":  { "type": "string" },
          "version": { "type": "string" }
        }
      }
    },
    "shell_integrations": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["path", "marker_start", "marker_end"],
        "properties": {
          "path":         { "type": "string", "description": "셸 설정 파일 절대 경로. 예: ~/.bashrc" },
          "marker_start": { "type": "string", "description": "삽입 블록 시작 마커. 예: # BEGIN OPENXGRAM" },
          "marker_end":   { "type": "string", "description": "삽입 블록 종료 마커. 예: # END OPENXGRAM" }
        }
      }
    },
    "external_resources": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["type", "id", "managed"],
        "properties": {
          "type":    { "type": "string", "description": "예: discord-webhook, telegram-bot, openagentx-registration" },
          "id":      { "type": "string", "description": "외부 서비스의 리소스 ID 또는 URL" },
          "managed": { "type": "boolean", "description": "true: 설치 시 Lifecycle Manager가 생성. false: 마스터 직접 생성." }
        }
      }
    },
    "registered_keys": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["alias", "address", "derivation_path"],
        "properties": {
          "alias":           { "type": "string" },
          "address":         { "type": "string" },
          "derivation_path": { "type": "string", "description": "예: m/44'/60'/0'/0/0" }
        }
      }
    },
    "ports": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["number", "protocol", "service"],
        "properties": {
          "number":   { "type": "integer" },
          "protocol": { "type": "string", "enum": ["tcp", "udp"] },
          "service":  { "type": "string" }
        }
      }
    },
    "os_keychain_entries": {
      "type": "array",
      "items": {
        "type": "object",
        "required": ["service", "account"],
        "properties": {
          "service": { "type": "string" },
          "account": { "type": "string" }
        }
      }
    },
    "selected_extractors": {
      "type": "object",
      "description": "코드 추출 LLM 어댑터 선택. 마스터 결정(2026-04-30).",
      "example": { "default": "gemini", "python": "claude" }
    },
    "inbound_webhook_port": {
      "type": "integer",
      "description": "inbound webhook 포트. 확정: 14921. 마스터 결정(2026-04-30).",
      "default": 14921
    },
    "backup_schedule": {
      "type": ["string", "null"],
      "description": "자동 백업 cron 표현식. 기본 null (미설정). 온보딩 Step 7에서 마스터 입력. 마스터 결정(2026-04-30).",
      "example": "0 18 * * *"
    }
  }
}
```

### 4.2 예시 (gcp-main, linux, primary)

```json
{
  "version": "1",
  "installed_at": "2026-04-30T14:32:00+09:00",
  "machine": {
    "alias": "gcp-main",
    "role": "primary",
    "os": "linux",
    "arch": "x86_64",
    "hostname": "starian-gcp",
    "tailscale_ip": "100.101.102.103"
  },
  "uninstall_token": "eyJhbGciOiJFZERTQSJ9.eyJtYWNoaW5lIjoiZ2NwLW1haW4iLCJpYXQiOjE3NDYwMDAwMDB9.SIGNATURE",
  "files": [
    {
      "path": "/home/llm/.starian/config.toml",
      "sha256": "a3f1b2c4d5e6...",
      "size_bytes": 512,
      "installed_at": "2026-04-30T14:32:01+09:00"
    }
  ],
  "directories": [
    { "path": "/home/llm/.starian", "created_by_installer": true },
    { "path": "/home/llm/.starian/models", "created_by_installer": true },
    { "path": "/home/llm/.starian/keystore", "created_by_installer": true }
  ],
  "system_services": [
    {
      "name": "openxgram-sidecar",
      "type": "systemd-user",
      "unit_file": "/home/llm/.config/systemd/user/openxgram-sidecar.service",
      "enabled": true,
      "started": true
    }
  ],
  "binaries": [
    {
      "path": "/home/llm/.local/bin/xgram",
      "sha256": "f9e8d7c6b5a4...",
      "version": "0.1.0.0"
    }
  ],
  "shell_integrations": [
    {
      "path": "/home/llm/.bashrc",
      "marker_start": "# BEGIN OPENXGRAM",
      "marker_end": "# END OPENXGRAM"
    }
  ],
  "external_resources": [
    {
      "type": "discord-webhook",
      "id": "https://discord.com/api/webhooks/123456/TOKEN",
      "managed": true
    }
  ],
  "registered_keys": [
    {
      "alias": "master",
      "address": "0xAbCd...EfGh",
      "derivation_path": "m/44'/60'/0'/0/0"
    }
  ],
  "ports": [
    { "number": 7300, "protocol": "tcp", "service": "xgram-rpc" },
    { "number": 7301, "protocol": "tcp", "service": "xgram-http" }
  ],
  "os_keychain_entries": [
    { "service": "openxgram", "account": "keystore-passphrase" }
  ],
  "selected_extractors": { "default": "gemini", "python": "claude" },
  "inbound_webhook_port": 14921,
  "backup_schedule": null
}
```

### 4.3 Drift 감지 알고리즘

manifest를 로드한 후 다음 순서로 실제 파일 시스템과 대조한다.

- files 배열 순회: 각 경로의 SHA256 해시를 현재 계산하여 manifest 값과 비교한다. 불일치 시 DRIFT 플래그.
- directories 배열 순회: 경로 존재 여부 확인. 없으면 MISSING 플래그.
- system_services 배열 순회: 서비스 파일 존재 여부와 enabled/started 상태 확인. 불일치 시 DRIFT 플래그.
- binaries 배열 순회: 경로 존재 및 SHA256 확인. 불일치 시 DRIFT 플래그.
- shell_integrations 배열 순회: 마커 블록 존재 여부 확인. 없으면 MISSING 플래그.
- ports 배열 순회: 각 포트의 점유 프로세스 PID 확인. xgram 프로세스가 아니면 CONFLICT 플래그.

DRIFT 또는 MISSING 항목이 있으면 uninstall 전에 마스터에게 목록을 출력하고 확인을 요청한다. 자동 무시 금지.

---

## 5. 제거 워크플로우 (xgram uninstall)

### 5.1 사전 검증

모든 검증이 통과한 후에만 5.2 백업 옵션 선택 단계로 진행한다.

단계 1 — manifest 존재 확인
- install-manifest.json을 읽는다. 없으면 오류 raise ("manifest를 찾을 수 없습니다. 수동 제거 안내: docs/manual-uninstall.md")

단계 2 — manifest 무결성 검증
- uninstall_token의 EdDSA 서명을 마스터 공개키로 검증한다.
- 서명 불일치 시 즉시 오류 raise. "manifest가 변조되었을 수 있습니다. 제거를 중단합니다."

단계 3 — drift 감지
- §4.3 알고리즘을 실행한다.
- DRIFT 또는 MISSING 항목 발견 시 목록 출력 후 마스터에게 계속 진행 여부를 묻는다.
- 자동으로 넘어가는 fallback 금지.

단계 4 — 외부 의존성 검사
- 이 사이드카의 포트나 API를 사용하는 다른 시스템이 있는지 확인한다. (등록된 OpenAgentX 피어, Tailscale 연결 피어 목록 출력)
- WARN 출력. 자동 차단은 하지 않으나 마스터에게 확인 후 진행.

---

### 5.2 백업 옵션 4가지

사전 검증 통과 후 백업 방식을 선택한다. 백업 검증이 완료된 후에만 5.4 제거 단계로 진행한다.

옵션 1 — Full backup (다른 사이드카에 sync)
- 대상 머신의 별칭 또는 Tailscale IP 입력
- Transport 레이어를 통해 전체 데이터를 대상 사이드카로 동기화
- 동기화 완료 후 대상 사이드카에서 확인 응답 수신
- 확인 응답이 없으면 제거 진행 중단

옵션 2 — Cold backup (암호화 zip)
- 백업 파일 생성: `~/.openxgram-backup-YYYYMMDD-HHMMSS.tar.gz.enc` (KST 기준)
- ChaCha20-Poly1305로 암호화 (마스터 결정 2026-04-30 — keystore §12.6 와 통일). 키: 패스워드(`XGRAM_KEYSTORE_PASSWORD`) → Argon2id KDF
- 백업 파일 SHA256 출력 (마스터가 기록 가능)
- 백업 파일 크기 및 위치 출력 후 마스터 확인

옵션 3 — 부분 보존 (특정 경로 명시)
- 마스터가 보존할 경로 목록 입력 (예: `~/.openxgram/sessions/`, `~/.openxgram/keystore/`)
- 지정 경로는 trash로 이동하지 않고 현재 위치에 유지
- manifest에 `preserved: true`로 기록

옵션 4 — 백업 없음
- 마스터가 정확히 `DELETE OPENXGRAM` 를 입력해야 한다 (대소문자 포함 정확 일치)
- 다른 문자열 입력 시 재입력 요청
- `--no-backup` 플래그 사용 시에도 동일한 확인 문자열 요구

---

### 5.3 명시적 확인

백업 완료 후 최종 확인 단계.

출력 내용:
- 제거될 파일 수, 디렉토리 수, 서비스 수 요약
- managed 외부 리소스 목록 (정리 예정)
- unmanaged 외부 리소스 목록 (정리 안 함)
- 보존 경로 목록 (옵션 3 선택 시)

확인 입력: `Y` 또는 `yes` (대소문자 무관). 다른 입력은 취소.

---

### 5.4 제거 단계 (역순 + 트랜잭션)

각 단계 실패 시 오류 raise. 완료된 단계는 manifest에 `removed_at` 타임스탬프(KST) 기록. 이후 재실행 시 `removed_at`이 있는 항목은 건너뛴다 (idempotent 보장).

단계 1 — 데몬 graceful shutdown
- 사이드카 데몬에 SIGTERM 전송
- 메시지 큐 flush 대기 (최대 30초)
- 30초 초과 시 SIGKILL. 이 경우 WARN 기록.
- 데몬 PID 소멸 확인 후 다음 단계 진행

단계 2 — managed 외부 리소스 정리
- `external_resources` 배열에서 `managed: true` 항목만 순회
- Discord webhook: `DELETE /webhooks/{id}` API 호출
- Telegram bot: 연결만 해제 (봇 자체 삭제는 Telegram에서 불가)
- OpenAgentX: 에이전트 등록 해제 API 호출
- `managed: false` 항목은 목록만 출력하고 절대 건드리지 않음

단계 3 — 시스템 서비스 등록 해제
- systemd-user: `systemctl --user disable --now {name}`, unit 파일 삭제, `systemctl --user daemon-reload`
- systemd-system: `systemctl disable --now {name}`, unit 파일 삭제 (sudo 필요)
- launchd-user: `launchctl unload {plist}`, plist 파일 삭제
- launchd-system: `launchctl unload {plist}`, plist 파일 삭제 (sudo 필요)
- windows-service: `sc.exe delete {name}` 또는 NSSM 제거

단계 4 — 셸 통합 정확 제거
- 각 `shell_integrations` 항목 순회
- `marker_start`와 `marker_end` 사이의 라인만 삭제
- 마커 라인 자체도 삭제
- 마커 바깥 코드는 절대 건드리지 않음
- sed를 사용한 정확 제거: `sed -i "/^# BEGIN OPENXGRAM$/,/^# END OPENXGRAM$/d"` 패턴

단계 5 — OS 키체인 항목 제거
- `os_keychain_entries` 배열 순회
- OS 키체인 라이브러리(keyring crate)를 통해 각 항목 삭제
- 항목이 없으면 (이미 삭제됨) 건너뜀 (오류 raise 금지)

단계 6 — 데이터 디렉토리 trash 이동
- `~/.openxgram/` 전체를 trash로 이동 (`rm -rf` 절대 금지)
- 구현: Rust `trash` 크레이트 사용 (Linux/macOS/Windows 추상화). 마스터 결정(2026-04-30).
  - Linux: XDG trash 스펙 구현 포함 (`trash` 크레이트)
  - macOS: native `NSFileManager.trashItem` API (`trash` 크레이트)
  - Windows: Recycle Bin API (`trash` 크레이트)
- 옵션 3으로 지정된 보존 경로는 trash 이동 전 다른 위치로 임시 이동 → trash 이동 후 원래 위치 복원
- 이동 완료 후 7일 후 영구 삭제 (trash-cli 자동 purge 또는 launchd/cron 등록)

단계 7 — 바이너리 삭제
- `binaries` 배열 순회
- 각 경로를 trash로 이동
- PATH에 등록된 심볼릭 링크도 함께 제거

단계 8 — install-manifest.json 삭제
- 모든 단계 완료 후 마지막으로 삭제
- manifest 삭제가 제거 완료의 공식 신호

---

### 5.5 사후 검증

흔적 검사를 통해 제거가 완전히 이루어졌는지 확인한다.

검사 범위: **홈 디렉토리만** (기본값). `--scan-system` 플래그 사용 시 시스템 전체로 확장. 마스터 결정(2026-04-30).

검사 패턴:
- `find $HOME -name "*xgram*"` (파일명 패턴)
- `find $HOME -name "*starian*"` (파일명 패턴)
- `find $HOME -name "*openxgram*"` (파일명 패턴)
- PATH에 `xgram` 명령 잔존 여부 확인
- systemd/launchd 서비스 목록에서 openxgram 잔존 여부 확인

결과 처리:
- 잔존 흔적 없음: "제거 완료. 흔적 없음 확인." 출력
- 잔존 흔적 발견: 목록을 마스터에게 출력. 자동 삭제 금지. "수동으로 확인 후 삭제하세요." 안내.

---

### 5.6 Idempotent 보장

`xgram uninstall`을 여러 번 실행해도 안전하다.

보장 메커니즘:
- 각 제거 단계 완료 시 manifest에 `removed_at` 타임스탬프(KST) 기록
- 재실행 시 `removed_at`이 있는 항목은 "이미 제거됨" 메시지 출력 후 건너뜀
- manifest 자체가 없으면 "이미 제거되었거나 설치된 적이 없습니다." 출력 후 종료 코드 0

부분 실패 복구:
- Step N에서 실패 후 재실행 시 Step 1~N-1은 idempotent하게 건너뜀
- Step N부터 다시 시작

---

## 6. doctor 점검 (xgram doctor)

### 6.1 점검 항목

각 항목은 OK / WARN / FAIL 중 하나로 판정되며 출력된다.

항목 1 — 사이드카 데몬 PID/uptime
- OK: 데몬이 실행 중이고 `/health` 응답 200
- WARN: 데몬은 실행 중이나 `/health` 응답 지연 (>500ms)
- FAIL: 데몬 미실행 또는 PID 없음
- --fix: 데몬 재시작 시도

항목 2 — 데이터 디렉토리 크기·여유
- OK: 사용률 < 80%
- WARN: 사용률 80~90%
- FAIL: 사용률 > 90%
- --fix: 30일 이상 지난 session 데이터 아카이브 제안 (자동 삭제 금지, 마스터 확인 후)

항목 3 — SQLite DB 무결성
- `PRAGMA integrity_check` 실행
- OK: "ok" 응답
- FAIL: 오류 내용 출력
- last vacuum 날짜 확인 (30일 초과 시 WARN)
- --fix: `VACUUM` 실행 (안전한 작업이므로 --fix 자동 허용)

항목 4 — Embedder 로드 상태
- BGE-small 모델 파일 존재 및 SHA256 검증
- OK: 파일 정상, 모델 로드 응답 확인
- WARN: 파일은 있으나 로드 응답 지연
- FAIL: 파일 없음 또는 해시 불일치
- --fix: 모델 재다운로드

항목 5 — Keystore 잠금 상태
- OK: keystore 파일 존재, 권한 600, 잠금 상태 (패스워드 미입력 상태)
- WARN: keystore 파일 권한이 600이 아님
- FAIL: keystore 파일 없음
- --fix: 권한 자동 수정 (`chmod 600`)

항목 6 — Tailscale 연결 + 피어
- Tailscale 비사용 설치: 건너뜀
- OK: `tailscale status` 실행 가능, 연결 상태 Up
- WARN: 연결 상태 Up이나 피어 수 0
- FAIL: tailscale 미실행 또는 연결 끊김
- --fix: `tailscale up` 실행 시도

항목 7 — Discord 봇 토큰 만료일
- Discord 비활성화 시: 건너뜀
- `GET /users/@me` 응답 확인
- OK: 응답 정상
- WARN: 응답 정상이나 토큰 만료 30일 이내 (Discord는 만료 없음, 봇 상태 변경 감지)
- FAIL: 401 응답 (토큰 만료 또는 봇 삭제)
- --fix: 토큰 재입력 안내

항목 8 — Telegram 봇 응답
- Telegram 비활성화 시: 건너뜀
- `getMe` API 호출
- OK: 응답 정상
- FAIL: 응답 오류 또는 타임아웃
- --fix: 토큰 재입력 안내

항목 9 — XMTP 클라이언트 연결
- XMTP 비사용 설치: 건너뜀
- OK: 클라이언트 초기화 및 네트워크 핑 정상
- FAIL: 연결 오류
- --fix: XMTP 클라이언트 재초기화

항목 10 — 디스크 사용률
- OK: 루트 파티션 사용률 < 85%
- WARN: 85~95%
- FAIL: > 95%
- --fix: 30일 이상 지난 trash 항목 purge 제안 (자동 삭제 금지)

항목 11 — 포트 점유 정상
- manifest의 ports 배열 순회
- OK: 각 포트를 xgram 프로세스가 점유
- FAIL: 포트 미점유 또는 다른 프로세스가 점유
- --fix: 데몬 재시작 시도 (포트 미점유 시)

항목 12 — 외부 어댑터 헬스
- 활성화된 어댑터 순회
- 각 어댑터 API 헬스체크 (최소 토큰 소비 방식)
- OK / WARN / FAIL 출력

항목 13 — 임베딩 모델 무결성
- 항목 4와 별도로 모델 파일의 전체 SHA256 재검증 (항목 4는 로드 상태, 항목 13은 파일 무결성)
- OK: SHA256 일치
- FAIL: 불일치 (파일 손상 가능성)
- --fix: 모델 재다운로드

### 6.2 출력 형식

```
xgram doctor 결과 (2026-04-30 14:32:00 KST)

[OK]   사이드카 데몬 PID=12345, uptime=3d 12h
[OK]   데이터 디렉토리 사용률 42% (2.1GB / 5.0GB)
[OK]   SQLite DB 무결성 정상, last vacuum 5일 전
[WARN] Embedder 로드 응답 지연 (823ms > 500ms)
[OK]   Keystore 잠금 상태 정상 (권한 600)
[OK]   Tailscale 연결 Up, 피어 3개
[FAIL] Discord 봇 응답 오류 (401 Unauthorized)
...

요약: 11 OK, 1 WARN, 1 FAIL
수정 가능 항목: xgram doctor --fix 실행
```

---

## 7. reset 모드 (xgram reset)

### 7.1 옵션 매트릭스

- `--test-only`: session prefix가 `test-` 인 세션 데이터만 삭제한다. 키, 설정, 일반 세션은 유지.
- `--hard`: init 직후 상태로 되돌린다. DB 데이터 전체 삭제, 설정 초기화. 키는 기본 유지 (--keep-keys 기본값).
- `--keep-keys`: 키스토어와 등록된 키페어를 유지한다. --hard와 함께 사용 시 키 외 모든 데이터 삭제.
- `--keep-config`: config.toml을 유지한다. --hard와 함께 사용 시 설정 외 모든 데이터 삭제.
- 인자 없음: 대화형 체크리스트 출력 후 마스터가 항목별 선택

대화형 예시:
```
reset 대상 선택 (스페이스로 선택, 엔터로 확인):
[x] test- 세션 데이터
[ ] 일반 세션 데이터
[ ] 임베딩 캐시
[ ] 어댑터 설정
[ ] 키스토어
```

### 7.2 reset 후 무결성

reset 완료 후 다음 항목들이 보존된다.

- DB 스키마: `schema_version` 테이블과 마이그레이션 이력은 항상 보존한다. 데이터만 삭제하고 스키마는 건드리지 않는다.
- install-manifest.json: 항상 보존한다. reset은 설치 기록을 지우지 않는다.
- 외부 어댑터 설정: `--keep-config` 플래그 사용 시 또는 대화형에서 선택 해제 시 보존.
- keystore: `--keep-keys` 플래그 사용 시 또는 대화형에서 선택 해제 시 보존.

reset 완료 후 `xgram doctor`를 자동 실행하여 상태를 확인한다.

---

## 8. migrate 정책 (xgram migrate)

### 8.1 버전 매트릭스

버전 명명: `MAJOR.MINOR.PATCH.BUILD`

자동 마이그레이션 (사용자 확인 불필요):
- `v0.1.0.x` → `v0.1.x.x`: PATCH 업그레이드. 스키마 변경 없거나 호환 가능한 컬럼 추가만.

마이그레이션 필요 (마스터 확인 후 진행):
- `v0.1.x` → `v0.2.x`: MINOR 업그레이드. 스키마 변경 가능. 마이그레이션 SQL 적용 필요.
- `v0.x` → `v1.x`: MAJOR 업그레이드. Breaking change. 수동 검토 권장.

skip-version 정책:
- 한 번에 하나의 MINOR 버전만 건너뛸 수 없다. 반드시 순차 적용.
- `v0.1` → `v0.3` 직접 마이그레이션 금지. `v0.1 → v0.2 → v0.3` 순서로 적용.
- `xgram migrate --to 0.3` 실행 시 내부적으로 0.2 마이그레이션 파일을 먼저 적용한다.

### 8.2 마이그레이션 단계

단계 1 — 데몬 정지
- `xgram stop` 실행 (graceful shutdown, §5.4 단계 1과 동일)

단계 2 — DB 백업
- 현재 DB를 `~/.openxgram/backup/pre-v{새버전}-{KST타임스탬프}.sqlite`로 복사
- 백업 파일 SHA256 계산 및 기록

단계 3 — 스키마 SQL 순차 적용
- 마이그레이션 파일 디렉토리에서 해당 버전의 SQL 파일 목록 로드
- 파일명 순서대로 적용 (예: `0002_add_vec_column.sql`, `0003_add_session_index.sql`)
- 각 SQL 파일 적용 전 `schema_migrations` 테이블에서 이미 적용된 버전 확인 (idempotent)
- 적용 완료 시 `schema_migrations` 테이블에 기록

단계 4 — install-manifest.json 업데이트
- `version` 필드를 새 버전으로 업데이트
- `migrated_at` 필드 추가 (KST ISO8601)

단계 5 — 바이너리 교체
- 새 바이너리를 다운로드하고 SHA256 검증
- 기존 바이너리를 `~/.openxgram/backup/xgram-v{이전버전}`으로 백업
- 새 바이너리로 교체

단계 6 — 데몬 재시작
- `xgram start` 실행

단계 7 — 헬스체크
- `GET /health` 응답 확인 (최대 10초 대기)
- 응답 없으면 오류 raise + 롤백 안내

### 8.3 롤백

`xgram migrate --rollback VERSION` 실행 시:

- 데몬 정지
- `~/.openxgram/backup/pre-v{VERSION}-*.sqlite` 최신 파일을 현재 DB로 복원
- install-manifest.json의 version 필드를 이전 버전으로 복원
- 이전 바이너리를 `~/.openxgram/backup/xgram-v{VERSION}`에서 복원
- 데몬 재시작 + 헬스체크

롤백 가능 조건: 백업 파일이 존재해야 한다. 백업 없이는 롤백 불가 (오류 raise).

---

## 9. CLI 명령어 전체

### xgram init

설명: 사이드카를 처음 설치하는 대화형 온보딩 마법사를 실행한다.

인자:
- 없음

옵션:
- `--config FILE`: TOML 또는 JSON 설정 파일로 비대화 모드 실행
- `--import SEED`: 다른 머신에서 동일 시드로 추가 인스턴스 설치
- `--dry-run`: 실제 변경 없이 수행될 작업 목록만 출력
- `--force`: 기존 설치가 있어도 덮어쓰기 (기존 데이터 손실 경고 출력)

예시:
- `xgram init` — 대화형 9단계 마법사 실행
- `xgram init --config /etc/xgram/config.toml` — CI 환경 비대화 설치
- `xgram init --import SEED --dry-run` — 시드 import 시 수행될 작업 미리 확인

종료 코드:
- 0: 설치 완료
- 1: 설치 실패 (오류 메시지 출력됨)
- 2: dry-run 완료 (실제 변경 없음)

---

### xgram status

설명: 사이드카 현재 상태를 출력한다.

인자:
- 없음

옵션:
- `--json`: JSON 형식으로 출력
- `--verbose`: 상세 정보 포함

예시:
- `xgram status` — 데몬 상태, 버전, 포트 출력
- `xgram status --json` — 스크립트 파이핑용 JSON 출력
- `xgram status --verbose` — 피어 목록, 어댑터 상태 포함

종료 코드:
- 0: 데몬 실행 중
- 1: 데몬 미실행
- 2: 설치되지 않음

---

### xgram doctor

설명: 설치 상태를 점검하고 건강 여부를 보고한다.

인자:
- 없음

옵션:
- `--fix`: 자동 수정 가능한 항목을 수정한다 (§6 참조)
- `--json`: JSON 형식으로 출력
- `--item ITEM`: 특정 항목만 점검 (예: `--item sqlite`, `--item embedder`)

예시:
- `xgram doctor` — 전체 항목 점검 출력
- `xgram doctor --fix` — 점검 후 수정 가능 항목 자동 수정
- `xgram doctor --item discord --fix` — Discord 어댑터만 점검 후 수정

종료 코드:
- 0: 모든 항목 OK
- 1: FAIL 항목 존재
- 2: WARN 항목만 존재 (FAIL 없음)

---

### xgram reset

설명: 사이드카를 선택한 범위만큼 초기 상태로 되돌린다.

인자:
- 없음

옵션:
- `--test-only`: test- 세션 데이터만 삭제
- `--hard`: init 직후 상태로 완전 초기화
- `--keep-keys`: 키스토어 유지
- `--keep-config`: config.toml 유지
- `--dry-run`: 실제 변경 없이 수행될 작업 목록만 출력

예시:
- `xgram reset --test-only` — 테스트 세션 데이터만 정리
- `xgram reset --hard --keep-keys` — 키 유지하고 나머지 전체 초기화
- `xgram reset` — 대화형 항목 선택

종료 코드:
- 0: reset 완료
- 1: reset 실패
- 2: dry-run 완료

---

### xgram migrate

설명: 새 버전으로 DB 스키마와 설정을 마이그레이션한다.

인자:
- 없음

옵션:
- `--to VERSION`: 목적 버전 지정 (기본: 최신)
- `--rollback VERSION`: 지정 버전으로 롤백
- `--dry-run`: 실제 변경 없이 수행될 작업 목록만 출력

예시:
- `xgram migrate` — 최신 버전으로 자동 마이그레이션
- `xgram migrate --to 0.2.0.0` — v0.2.0.0으로 마이그레이션
- `xgram migrate --rollback 0.1.0.0` — v0.1.0.0으로 롤백

종료 코드:
- 0: 마이그레이션 완료
- 1: 마이그레이션 실패
- 2: dry-run 완료

---

### xgram uninstall

설명: 사이드카를 완전히 제거한다.

인자:
- 없음

옵션:
- `--backup-to MACHINE_ALIAS`: 다른 사이드카에 Full backup 후 제거 (옵션 1)
- `--no-backup`: 백업 없이 제거. 확인 문자열 `DELETE OPENXGRAM` 입력 필요.
- `--keep-data PATH`: 지정 경로 보존 (반복 사용 가능). 예: `--keep-data ~/.openxgram/sessions/`
- `--dry-run`: 실제 변경 없이 수행될 작업 목록만 출력
- `--scan-system`: 흔적 검사를 홈 디렉토리(기본)에서 시스템 전체로 확장. 마스터 결정(2026-04-30).

예시:
- `xgram uninstall` — 대화형 백업 옵션 선택 후 제거
- `xgram uninstall --backup-to macmini-home` — macmini-home 사이드카에 sync 후 제거
- `xgram uninstall --no-backup --dry-run` — 백업 없이 제거 시 수행될 작업 미리 확인

종료 코드:
- 0: 제거 완료 또는 이미 제거됨
- 1: 제거 실패
- 2: dry-run 완료

---

## 10. TUI/GUI 설치 마법사

### 10.1 ASCII 와이어프레임 (9단계 화면)

```
┌─────────────────────────────────────────────────────┐
│  OpenXgram 설치 마법사                    [1/9]     │
│  ─────────────────────────────────────────────────  │
│                                                     │
│  사전 점검 중...                                    │
│                                                     │
│  [OK]  디스크 여유: 12.3 GB                        │
│  [OK]  포트 7300: 미점유                           │
│  [OK]  포트 7301: 미점유                           │
│  [OK]  홈 디렉토리 쓰기 권한                       │
│  [OK]  OS: Linux (x86_64)                          │
│                                                     │
│  진행률 ████████░░░░░░░░░░░░ 11%                   │
│                                                     │
│  [엔터] 다음  [q] 종료                             │
└─────────────────────────────────────────────────────┘
```

```
┌─────────────────────────────────────────────────────┐
│  OpenXgram 설치 마법사                    [3/9]     │
│  ─────────────────────────────────────────────────  │
│                                                     │
│  마스터 시드                                        │
│                                                     │
│  ○ 새 시드 생성 (권장)                              │
│  ● 기존 시드 import                                 │
│                                                     │
│  24단어 입력:                                       │
│  ┌───────────────────────────────────────────────┐ │
│  │ word1 word2 word3 word4 word5 word6 word7     │ │
│  │ word8 word9 word10 ... (입력 중)              │ │
│  └───────────────────────────────────────────────┘ │
│                                                     │
│  진행률 ████████████████░░░░ 33%                   │
│                                                     │
│  [엔터] 확인  [b] 이전  [q] 종료                   │
└─────────────────────────────────────────────────────┘
```

### 10.2 키바인딩

- `엔터 / →`: 다음 단계
- `b / ←`: 이전 단계 (이미 완료된 단계는 재진입 가능)
- `q / Ctrl+C`: 마법사 종료 (부분 완료된 설치는 자동 롤백)
- `스페이스`: 체크박스 토글 (다중 선택 항목)
- `Tab`: 필드 간 이동
- `Ctrl+Z`: undo (텍스트 입력 중)
- `?`: 현재 단계 도움말

### 10.3 진행률 바 형식

```
진행률 ████████████░░░░░░░░░░ 55%  [6/9] 데이터베이스 초기화
```

- 블록 문자: `█` (완료), `░` (미완료)
- 너비: 20 블록 고정
- 퍼센트와 현재 단계명 함께 표시

### 10.4 GUI Tauri 컴포넌트 구조 (Phase 2)

루트 컴포넌트 구조:
- `<InstallWizard>`: 전체 마법사 컨테이너. 현재 step 상태 관리.
  - `<StepHeader>`: 단계 번호 + 진행률 바
  - `<StepContent>`: 각 단계별 폼 컴포넌트 (동적 렌더링)
    - `<StepPreCheck>`: Step 1 — 점검 결과 목록
    - `<StepMachineIdentity>`: Step 2 — 별칭/역할 입력
    - `<StepSeed>`: Step 3 — BIP39 입력 (보안: 화면 마스킹)
    - `<StepKeyPair>`: Step 4 — 패스워드 입력
    - `<StepDataDir>`: Step 5 — 경로 선택 (파일 다이얼로그)
    - `<StepDatabase>`: Step 6 — 다운로드 진행 표시
    - `<StepAdapters>`: Step 7 — 어댑터별 토글 + 입력
    - `<StepTransport>`: Step 8 — Transport 선택
    - `<StepDaemon>`: Step 9 — 서비스 등록 확인
  - `<StepNavigation>`: 이전/다음 버튼

---

## 11. OS별 차이

### Linux

서비스 등록 방식:
- user-level (기본): `~/.config/systemd/user/openxgram-sidecar.service`
  ```
  systemctl --user enable --now openxgram-sidecar
  loginctl enable-linger $USER  (재부팅 후 자동 시작을 위해)
  ```
- system-level (root 필요): `/etc/systemd/system/openxgram-sidecar.service`
  ```
  systemctl enable --now openxgram-sidecar
  ```

셸 통합 마커:
```bash
# BEGIN OPENXGRAM
export PATH="$HOME/.local/bin:$PATH"
alias xgram="$HOME/.local/bin/xgram"
# END OPENXGRAM
```

대상 파일: `~/.bashrc`, `~/.zshrc`, `~/.config/fish/config.fish`

trash 라이브러리: `trash-cli` (`trash-put` 명령). 설치 확인 후 없으면 XDG trash 수동 구현.

---

### macOS

서비스 등록 방식:
- user-level LaunchAgent (기본): `~/Library/LaunchAgents/us.openxgram.sidecar.plist`
  ```
  launchctl load -w ~/Library/LaunchAgents/us.openxgram.sidecar.plist
  ```
- system-level LaunchDaemon (sudo 필요): `/Library/LaunchDaemons/us.openxgram.sidecar.plist`
  ```
  sudo launchctl load -w /Library/LaunchDaemons/us.openxgram.sidecar.plist
  ```

plist 핵심 키: `Label`, `ProgramArguments`, `RunAtLoad`, `KeepAlive`, `StandardOutPath`, `StandardErrorPath`

셸 통합 마커: Linux와 동일 형식.

trash: macOS native trash API (`NSFileManager.trashItem`). Rust에서 `trash` 크레이트 사용.

OS 키체인: Keychain Services API. Rust에서 `keyring` 크레이트 사용 (`service: "openxgram"`, `account: "keystore-passphrase"`).

---

### Windows

서비스 등록 방식:
- Windows Service via NSSM (Non-Sucking Service Manager)
  ```
  nssm install openxgram-sidecar "C:\Users\{USER}\.local\bin\xgram.exe" "start"
  nssm start openxgram-sidecar
  ```
- 또는 sc.exe (NSSM 없는 환경)
  ```
  sc.exe create openxgram-sidecar binPath="..." start=auto
  sc.exe start openxgram-sidecar
  ```

셸 통합:
- PowerShell 프로파일 (`$PROFILE`)에 마커 삽입
  ```powershell
  # BEGIN OPENXGRAM
  $env:PATH = "$env:USERPROFILE\.local\bin;" + $env:PATH
  # END OPENXGRAM
  ```
- cmd.exe: 레지스트리 `HKCU\Environment\PATH` 직접 수정 (마커 기반 정확 제거 불가 — WARN 출력)

trash: Recycle Bin API (`SHFileOperationW` with `FO_DELETE` + `FOF_ALLOWUNDO`). Rust에서 `trash` 크레이트 사용.

OS 키체인: Windows Credential Manager. Rust에서 `keyring` 크레이트 사용.

---

### 셸 통합 정확 제거 공통 규칙

bash/zsh:
```bash
sed -i "/^# BEGIN OPENXGRAM$/,/^# END OPENXGRAM$/d" ~/.bashrc
```

fish:
```fish
# fish는 sed 대신 파이썬 또는 Rust에서 라인 파싱으로 제거
```

PowerShell:
```powershell
(Get-Content $PROFILE) | Where-Object { $_ -notmatch "# (BEGIN|END) OPENXGRAM" -and -not ($inBlock) } | Set-Content $PROFILE
```

마커 외 라인은 어떤 경우에도 건드리지 않는다.

---

## 11.5 에러 처리 (fallback 금지 적용)

모든 에러는 raise다. 조용히 무시하는 경로는 없다.

Silent Error 4패턴 필수 적용:
- reqwest: 모든 HTTP 호출에 `.error_for_status()?` 강제. XMTP REST API 포함 예외 없음.
- rusqlite: `execute()` 후 `affected_rows()` 검증. UPDATE/DELETE 0건 → raise.
- tokio-cron-scheduler: job에 panic 핸들러 + tracing 로깅 필수. job panic silent 흡수 금지.
- keyring: 저장 후 `get()` round-trip 검증. headless Linux silent 실패 → raise.

init/uninstall/doctor/reset 모든 워크플로우에 동일하게 적용한다.

---

## 12. 보안

### 12.1 uninstall_token

- 마스터 시드에서 파생된 EdDSA(Ed25519) 개인키로 서명한다.
- 서명 대상 메시지: `machine.alias + installed_at + machine.hostname` (UTF-8 바이트 이어붙이기)
- 서명 결과를 base64url 인코딩하여 manifest에 저장한다.
- 제거 전 마스터 공개키로 서명을 검증한다. 검증 실패 시 즉시 오류 raise.
- manifest 파일이 다른 머신으로 복사되어 악용되는 것을 방지한다 (hostname 포함).

### 12.1.1 BIP39 시드 백업 정책 (마스터 결정 2026-04-30)

- 신규 생성 시 24단어를 화면에 표시한다.
- 마스터가 Y/N으로 기록 여부를 확인한다.
- 재입력 옵션(권장, 강제 아님): `xgram init --verify-seed` 플래그로 24단어 재입력 검증 지원.
- N 입력 시 설치 종료. 백업 없이는 진행 불가.

### 12.1.2 다른 머신 sync 인증 (마스터 결정 2026-04-30)

- 1차 방어: Tailscale mTLS (Tailscale 사용 환경에서 자동 적용)
- 2차 방어: 시드 서명 챌린지-응답 (두 머신이 같은 시드 보유 시 상호 인증)
- 이중 방어 모두 통과해야 sync 진행. 어느 하나라도 실패 시 즉시 reject.

### 12.2 셸 통합 정확 제거

- 반드시 `# BEGIN OPENXGRAM` / `# END OPENXGRAM` 마커 쌍으로 블록을 감싼다. 마스터 결정(2026-04-30).
- 제거 시 마커와 마커 사이의 라인만 삭제한다. 1바이트도 다른 코드를 건드리지 않는다.
- 마커가 없는 경우 (수동으로 이미 제거됨): 건너뜀 (오류 raise 금지).
- 마커가 하나만 있는 경우 (불완전한 상태): WARN 출력 + 마스터에게 수동 확인 안내.

### 12.3 managed 외부 리소스만 정리

- `managed: true` 리소스만 정리한다. Lifecycle Manager가 설치 시 생성한 것만.
- `managed: false` 리소스는 목록만 출력하고 절대 건드리지 않는다.
- 마스터가 설치 전부터 가지고 있던 Discord 봇, Telegram 봇, Notion 연동 등은 unmanaged.

### 12.4 trash 사용 원칙

- `rm -rf` 명령은 어떤 경우에도 사용하지 않는다.
- 모든 파일 삭제는 OS trash API를 통해 이루어진다.
- trash 이동 후 7일이 지나면 영구 삭제된다 (trash purge). 즉시 영구 삭제는 마스터 명시적 요청 시에만.
- `--no-backup` 옵션 사용 시에도 trash를 사용한다.

### 12.5 검증 실패 시 즉시 raise

- 모든 검증 단계에서 실패는 즉시 오류를 raise한다.
- 오류를 무시하고 다음 단계로 진행하는 fallback 로직을 구현하지 않는다.
- 오류 메시지는 한국어와 영어 병기, 원인 명시, 다음 조치 안내를 포함한다.
- 마스터 알림 채널(Discord 또는 Telegram)이 활성화되어 있으면 오류 발생 시 알림 전송.

### 12.6 대칭 암호화 — ChaCha20-Poly1305 (마스터 결정 2026-04-30)

- 대칭 암호화 알고리즘: ChaCha20-Poly1305 확정. AES-GCM 제외.
- 제외 사유: AES-GCM은 AES-NI 없는 ARM 환경에서 성능이 크게 저하된다. ChaCha20-Poly1305는 ARM/x86 멀티 환경에서 균일한 성능을 보장한다.
- 적용 범위: keystore 암호화, cold backup 암호화, vault layer 암호화 모두 ChaCha20-Poly1305 사용.
- 크레이트: `chacha20poly1305`.

### 12.7 keyring round-trip 검증 (Silent Error 방지)

- keyring 저장 직후 반드시 `get()`으로 round-trip 검증한다.
- headless Linux 환경(Secret Service 미설치)에서 저장이 silently 실패하는 이력이 있다.
- 검증 실패 시 즉시 raise + 마스터에게 keyring 환경 설정 안내.

---

## 13. 테스트 시나리오

### 시나리오 A — 라운드트립 (설치 → 헬스체크 → 제거 → 흔적 0건)

전제조건: 클린 환경 (설치된 흔적 없음)

단계:
- `xgram init` 실행 후 9단계 완료
- `xgram doctor` 실행 → 모든 항목 OK 확인
- `xgram uninstall --no-backup` 실행 → `DELETE OPENXGRAM` 입력
- 흔적 검사: `find $HOME -name "*xgram*" -o -name "*starian*"` 결과 0건 확인

합격 기준: 흔적 0건. doctor 결과 전 항목 OK. uninstall 종료 코드 0.

---

### 시나리오 B — install → reset --hard → 즉시 재사용 가능

단계:
- `xgram init` 완료
- `xgram reset --hard --keep-keys` 실행
- `xgram doctor` 실행 → 모든 항목 OK 확인
- 세션 생성 테스트 (사이드카가 정상 동작하는지)

합격 기준: reset 후 doctor OK. 세션 생성 성공.

---

### 시나리오 C — dry-run 정확성

단계:
- `xgram init --dry-run` 실행 → 작업 목록 출력 확인
- 실제 파일 시스템 변경 없음 확인 (`ls ~/.openxgram` 실패 또는 존재하지 않음)
- `xgram uninstall --dry-run` 실행 → 작업 목록 출력 확인
- 실제 파일 변경 없음 확인

합격 기준: dry-run 후 파일 시스템 상태 불변. 종료 코드 2.

---

### 시나리오 D — idempotent: uninstall 2회 연속 안전

단계:
- `xgram init` 완료
- `xgram uninstall --no-backup` 실행 (1회)
- `xgram uninstall --no-backup` 즉시 재실행 (2회)

합격 기준: 2회 모두 종료 코드 0. 오류 없음. 2회차는 "이미 제거되었습니다." 출력.

---

### 시나리오 E — 마스터 반복 워크플로우 (install → 사용 → uninstall 10회)

단계:
- 다음 루프를 10회 실행:
  - `xgram init --config /tmp/test-config.toml`
  - 세션 생성 1개 (`xgram session new "test"`)
  - `xgram doctor --json` → FAIL 항목 0건 확인
  - `xgram uninstall --no-backup`
  - `find $HOME -name "*xgram*"` → 0건 확인

합격 기준: 10회 모두 성공. 각 회차 후 흔적 0건.

---

### 시나리오 F — 부분 실패 복구

단계:
- `xgram init` 완료
- `xgram uninstall` 실행 중 Step 4 (셸 통합 제거) 단계에서 강제 종료 (`kill -9`)
- `xgram uninstall` 재실행
- Step 1~3은 idempotent하게 건너뛰는지 확인 (already removed 메시지)
- Step 4부터 재시작하여 완료되는지 확인

합격 기준: 재실행 시 이미 완료된 단계 건너뜀. 최종 제거 완료. 흔적 0건.

---

### 시나리오 G — drift detection

단계:
- `xgram init` 완료
- manifest에 기록된 파일 중 하나를 수동으로 수정 (SHA256 변경)
- `xgram uninstall` 실행 → drift 감지 경고 출력 확인
- 마스터가 계속 진행 선택 시 정상 제거 완료

합격 기준: drift 감지 후 마스터 확인 요청. 확인 후 정상 제거.

---

### 시나리오 H — 다른 머신으로 sync 후 제거 (Full backup)

전제조건: 두 번째 머신에 xgram 실행 중

단계:
- 첫 번째 머신에서 `xgram uninstall --backup-to second-machine` 실행
- 동기화 완료 및 두 번째 머신 확인 응답 수신 확인
- 첫 번째 머신 제거 완료 확인
- 두 번째 머신에서 동기화된 데이터 접근 가능 확인

합격 기준: sync 완료 확인 후 제거 진행. 두 번째 머신 데이터 정상.

---

### 시나리오 I — cold backup → 다른 머신에서 복원

단계:
- `xgram uninstall` 실행 → 백업 옵션 2 (Cold backup) 선택
- `~/.openxgram-backup-*.tar.gz.enc` 생성 확인
- 백업 파일을 다른 머신으로 복사
- 다른 머신에서 `xgram init --import-backup ~/.openxgram-backup-*.tar.gz.enc` 실행 (Phase 1.5)
- 복원된 세션 데이터 접근 가능 확인

합격 기준: 백업 파일 생성. 다른 머신에서 복원 성공. 세션 데이터 접근 가능.

---

## 14. Phase 분할

### 14.1 Phase 1 MVP (4~5일 목표)

구현 대상:
- `xgram init`: 9단계 대화형 마법사 (TUI, §10)
- install-manifest.json 생성·서명·검증
- `xgram uninstall`: 백업 옵션 1 (Full backup via sync) + 옵션 2 (Cold backup)
- `xgram doctor`: §6의 핵심 항목 1~7
- `xgram reset`: `--test-only` + `--hard`
- `xgram status`: 기본 상태 출력
- Linux + macOS 지원

제외:
- Windows 지원
- GUI 마법사
- `xgram migrate`
- doctor `--fix`
- Cold backup restore (`--import-backup`)

---

### 14.2 Phase 1.5

구현 대상:
- `xgram init --config FILE`: 비대화 모드 (CI 지원)
- `xgram migrate` 기본: v0.1.x → v0.2.x
- `xgram doctor --fix`: 자동 수정 가능 항목
- Cold backup restore: `xgram init --import-backup`
- doctor 항목 8~13 추가

---

### 14.3 Phase 2

구현 대상:
- GUI 마법사 (Tauri, §10.4)
- Windows Service 지원 (NSSM)
- 라운드트립 자동 테스트 통합 (시나리오 A~I 자동화)
- doctor 전체 항목 완성
- `xgram migrate --rollback` 완전 구현

---

## 15. 의존성

### 15.1 내부 모듈 의존성

- keystore 모듈: 시드 로드, EdDSA 서명/검증, keystore 패스워드 암호화(ChaCha20-Poly1305). uninstall_token 생성/검증에 필수.
- DB 마이그레이션: rusqlite 기반 자체 마이그레이션 러너. sqlx-cli 제외. §8의 순차 SQL 적용에 사용.
- 외부 어댑터 모듈: Discord/Telegram 토큰 검증 API 호출. Step 7 및 doctor 항목 7~8에 사용.

### 15.2 외부 크레이트

4결정 적용 크레이트 (마스터 결정 2026-04-30):
- `rusqlite` + `rusqlite-vec`: SQLite DB 드라이버 + sqlite-vec 통합. sqlx 제외.
- `fastembed` (multilingual-e5-small): 임베딩 모델. 560MB. 한국어/영어 지원.
- `chacha20poly1305`: 대칭 암호화. AES-GCM 대신 사용. ARM/x86 균일 성능.
- `reqwest`: XMTP 노드 REST API 직접 호출. 공식 Rust SDK 부재(libxmtp WASM 우선). 모든 호출에 `.error_for_status()?` 강제.
- `tokio-cron-scheduler`: 야간 reflection 스케줄러. job panic 핸들러 필수 (silent 흡수 방지).
- `keyring`: OS 키체인 추상화. 저장 후 round-trip get 검증 필수 (headless Linux silent 실패 방지).

기타 크레이트:
- `trash`: **Rust `trash` 크레이트** — Linux/macOS/Windows 크로스 OS 추상화. 외부 시스템 패키지(trash-cli) 의존 없음. 마스터 결정(2026-04-30).
- `sysinfo`: 프로세스 목록 및 포트 점유 확인
- `indicatif`: TUI 진행률 바
- `dialoguer`: TUI 대화형 입력 (선택, 패스워드, 체크박스)
- `bip39`: BIP39 니모닉 생성 및 검증
- `ed25519-dalek`: EdDSA 서명/검증
- `scrypt`: keystore KDF

### 15.3 OS 추상화

- systemd 관리: `systemctl` CLI 호출 (D-Bus 직접 연결은 Phase 2)
- launchd 관리: `launchctl` CLI 호출
- Windows Service: NSSM CLI 호출 또는 `windows-service` 크레이트

---

## 16. 결정 필요 항목 — 전체 확정 (2026-04-30)

모든 항목 마스터 결정 완료. 미결 사항 없음.

결정 1 — 데이터 디렉토리 기본 경로: **`~/.openxgram/`** 확정. manifest, 문서, 백업 파일명 모두 이 경로 기준.

결정 2 — OS 서비스 등록 기본값: **대화형 `[Y/n]` 확인, 기본 Y** 확정. 엔터 입력 시 자동 등록. Step 9 참조.

결정 3 — BIP39 시드 백업 강제 여부: **24단어 표시 + Y/N 확인 + 재입력 옵션 권장(강제 아님)** 확정. 12.1.1절 참조.

결정 4 — Linux trash 라이브러리: **Rust `trash` 크레이트** 확정. 외부 시스템 패키지(trash-cli) 의존 없음. 크로스 OS 지원.

결정 5 — 흔적 검사 범위: **홈 디렉토리만** (기본). `--scan-system` 플래그로 시스템 전체 확장 가능.

결정 6 — backup 보관 위치: **`~/.openxgram/backup/`** (데이터 디렉토리 내) 확정. uninstall 시 backup도 함께 trash 이동됨에 유의.

결정 7 — 다른 머신 sync 인증: **Tailscale mTLS (1차) + 시드 서명 챌린지-응답 (2차) 이중 방어** 확정. 12.1.2절 참조.

---

*문서 끝*
