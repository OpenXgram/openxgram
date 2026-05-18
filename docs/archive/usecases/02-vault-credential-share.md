# 02 — Vault 로 API 키 안전 공유

> 한 줄 요약: ChaCha20-Poly1305 로 암호화된 vault 에 API 키를 저장하고, ACL 로 다른 에이전트가 정해진 한도 안에서만 꺼내 쓰게 한다.

## 시나리오

마스터는 OpenAI / Discord / Telegram 등 29 개 외부 서비스 키를 한 곳에 넣어두고, AI 에이전트(`ai-poster`)에게는 SNS 토큰만 일일 30 회 한도로 노출하고 싶다. 평문 .env 노출은 금지. 암호화된 vault 에 저장하고 ACL 로 행위·일일 한도·정책(auto/confirm/mfa)을 분리한다.

## 사전 준비

- `xgram init` 완료
- `XGRAM_KEYSTORE_PASSWORD` export
- 키를 호출할 에이전트 식별자 합의 (예: `ai-poster`)

## 단계별 명령 시퀀스

```bash
# 1) Discord webhook 저장 (sns 태그)
xgram vault set \
  --key DISCORD_WEBHOOK_URL \
  --value "https://discord.com/api/webhooks/..." \
  --tags sns,discord,outbound

# 2) Telegram bot token 저장
xgram vault set \
  --key TELEGRAM_BOT_TOKEN \
  --value "7869683671:..." \
  --tags sns,telegram

# 3) 메타데이터 list (값은 노출 안 함)
xgram vault list

# 4) ACL — ai-poster 는 sns 키만 get 가능, 일일 30 회, auto 승인
xgram vault acl-set \
  --key-pattern "DISCORD_*" \
  --agent ai-poster \
  --actions get \
  --daily-limit 30 \
  --policy auto

xgram vault acl-set \
  --key-pattern "TELEGRAM_*" \
  --agent ai-poster \
  --actions get \
  --daily-limit 30 \
  --policy auto

# 5) ACL list 로 정책 확인
xgram vault acl-list

# 6) (필요 시) 민감 키는 confirm 정책 — pending 큐를 통해 마스터 승인
xgram vault acl-set \
  --key-pattern "OPENAI_*" \
  --agent eno \
  --actions get \
  --daily-limit 5 \
  --policy confirm

# 7) (mfa 정책) TOTP secret 발급
xgram vault mfa-issue --agent ai-poster

# 8) confirm 큐 처리 (마스터 측)
xgram vault pending
xgram vault approve <PENDING_ID>
xgram vault deny <PENDING_ID>

# 9) 평문 한 번 꺼내보기 (master 권한)
xgram vault get --key DISCORD_WEBHOOK_URL

# 10) 키 회수
xgram vault delete --key OLD_TOKEN
xgram vault acl-delete --key-pattern "DISCORD_*" --agent ai-poster
```

## 기대 결과

```
$ xgram vault list
key                       tags                        updated_at (KST)
DISCORD_WEBHOOK_URL       sns,discord,outbound        2026-05-04 14:21:08+09:00
TELEGRAM_BOT_TOKEN        sns,telegram                2026-05-04 14:21:33+09:00

$ xgram vault acl-list
key_pattern    agent       actions   daily_limit   policy   used_today
DISCORD_*      ai-poster   get       30            auto     0
TELEGRAM_*     ai-poster   get       30            auto     0
OPENAI_*       eno         get       5             confirm  0
```

## 주의점

- **fallback 금지**: ACL 조회 실패·한도 초과 시 묵시적 0 반환 금지 — raise. 호출자가 명시적으로 처리한다.
- **롤백 가능**: `vault delete` 는 영구 삭제. 삭제 전 `vault get` 으로 백업 후 진행 권장. ACL 변경은 즉시 효력.
- **DB 변경 승인**: `vault set/delete`, `acl-set/acl-delete` 는 keystore unwrap 이 필요하므로 사실상 마스터만 가능. confirm 정책은 마스터 승인 큐를 강제한다.
- **KST 시간대**: `daily_limit` 의 일자 경계, audit 로그 timestamp 모두 Asia/Seoul 자정 기준.
- Phase 1 enforcement: `policy=auto` 만 실 적용. confirm/mfa 는 큐·TOTP 발급까지는 동작하나 실제 차단 enforcement 는 후속 PR.

## 관련 PRD

- `docs/prd/PRD-OpenXgram-v1.md` §8 Vault (ChaCha20-Poly1305)
- `docs/prd/PRD-OpenXgram-v1.md` §8.4 ACL · 일일 한도 · 3 단계 정책
