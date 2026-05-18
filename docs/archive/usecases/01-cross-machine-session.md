# 01 — 여러 머신에서 동일 세션 이어가기

> 한 줄 요약: 한 머신에서 시작한 대화·기억을 시드 import + session export/import 로 다른 머신에서 그대로 이어간다.

## 시나리오

마스터는 평소 GCP 머신(`gcp-main`)에서 작업하다가 노트북(`mac-laptop`)으로 이동한다. GCP 에서 진행 중이던 PRD 검토 세션의 메시지·episode 를 노트북에서 그대로 이어받아 동일한 컨텍스트로 작업을 계속해야 한다. master 키는 한 개여야 하므로 노트북은 새 키를 만들지 않고 GCP 의 시드를 import 하고, session 단위로 text-package-v1 JSON 을 export/import 한다.

## 사전 준비

- GCP 머신: 이미 `xgram init --alias gcp-main --role primary` 완료
- 노트북: OpenXgram 미설치 (clean slate)
- 두 머신 모두 동일한 master 키를 공유해야 함 (PRD §17 master_public_key 검증)
- 환경변수 `XGRAM_KEYSTORE_PASSWORD` 가 양쪽에 동일하게 export 되어 있어야 함

## 단계별 명령 시퀀스

```bash
# 1) GCP — master 시드 export (BIP39 mnemonic 출력)
xgram keypair export --name master

# 2) 노트북 — 환경변수에 시드 주입 후 import 모드 init
export XGRAM_SEED="word1 word2 ... word24"
export XGRAM_KEYSTORE_PASSWORD="<GCP 와 동일한 password>"
xgram init --alias mac-laptop --role secondary --import

# 3) GCP — 진행 중 session ID 확인
xgram session list

# 4) GCP — session 통째로 export (text-package-v1 JSON)
xgram session export --session-id <SESSION_ID> --out /tmp/session.json

# 5) /tmp/session.json 을 노트북으로 안전 전송 (scp/rsync)
scp /tmp/session.json mac-laptop:/tmp/session.json

# 6) 노트북 — verify 모드로 import (master_public_key 서명 검증)
xgram session import --input /tmp/session.json --verify

# 7) 노트북 — import 된 session 확인
xgram session list
xgram session show <NEW_SESSION_ID>

# 8) 노트북에서 메시지 추가 (기존 컨텍스트 위에서 이어가기)
xgram session message --session-id <NEW_SESSION_ID> --sender master --body "노트북에서 이어서 — Phase 2 검토 시작"
```

## 기대 결과

```
$ xgram session import --input /tmp/session.json --verify
✓ session import 완료
  source         : /tmp/session.json
  new_session_id : 01HZ9XK4M2C7Q9R3T8N5V6P0YZ
  messages       : 24
  episodes       : 3
  signatures_ok  : 24/24
```

## 주의점

- **fallback 금지**: `--verify` 누락 시 master_public_key 검증을 건너뛰지 말고 명시적으로 호출한다. 검증 실패는 raise — 조용히 임포트 금지.
- **롤백 가능**: import 는 새 session_id 로 들어가므로 안전. 원본은 GCP 에 그대로 남는다.
- **DB 변경 승인**: `session export/import` 는 자기 데이터 한정이라 자동 가능. 단, `--import` 모드 init 은 keystore 를 새로 쓰므로 마스터 명령 필요.
- **KST 시간대**: export JSON 의 timestamp 는 Asia/Seoul 기준 ISO-8601 — UTC 변환 금지.
- 노트북 password 가 GCP 와 다르면 keystore unwrap 실패 → raise. 동일 password 강제.

## 관련 PRD

- `docs/prd/PRD-OpenXgram-v1.md` §17 Memory Transfer (text-package-v1)
- `docs/prd/PRD-OpenXgram-v1.md` §20 F (verify import)
