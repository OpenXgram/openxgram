# 03 — USDC 자동 결제 + MFA

> 한 줄 요약: USDC on Base 결제 intent 를 만들고 master ECDSA 로 서명한 뒤, 외부 도구(예: cast/foundry)로 제출하고 상태를 mark-submitted / mark-confirmed 로 진행시킨다.

## 시나리오

Akashic 가 마스터 대신 자동으로 외부 서비스 비용을 USDC 로 정산해야 한다. OpenXgram 은 결제 intent 의 생성·서명·상태 추적까지 책임지고, 실제 트랜잭션 broadcast 는 외부 도구가 수행한다 (xgram 자체는 RPC 를 들지 않는다 — 보안 분리). 마스터는 모든 intent 를 list 로 추적하고 큰 금액에는 password (= MFA) 를 요구한다.

## 사전 준비

- `xgram init` 완료, `keypair` 에 master 키 존재
- `XGRAM_KEYSTORE_PASSWORD` export — `payment sign` 에 매번 필요
- 외부 broadcast 도구 준비 (예: foundry `cast send`)

## 단계별 명령 시퀀스

```bash
# 1) 지원 chain 확인
xgram payment chains

# 2) 결제 intent 생성 — 25 USDC 를 0xRecipient 로 송금
xgram payment new \
  --amount 25.00 \
  --chain base \
  --to 0x000000000000000000000000000000000000dEaD \
  --memo "akashic monthly compute fee 2026-05"

# 3) intent list — 새로 생긴 ID 확인
xgram payment list

# 4) 단건 상세
xgram payment show <PAYMENT_ID>

# 5) master ECDSA 서명 (MFA = keystore password 요구)
xgram payment sign --id <PAYMENT_ID>

# 6) 외부에서 broadcast — 예: cast send (이 단계는 xgram 외부)
#    cast send <USDC_BASE_CONTRACT> "transfer(address,uint256)" 0x...dEaD 25000000 \
#      --rpc-url $BASE_RPC --private-key $MASTER_KEY
#    → tx_hash 0xabc... 획득

# 7) submitted 상태로 mark — block 확정 전 단계
xgram payment mark-submitted \
  --id <PAYMENT_ID> \
  --tx-hash 0xabc...

# 8) block 확정 후 confirmed 로 mark
xgram payment mark-confirmed --id <PAYMENT_ID>

# 실패 케이스 — gas 부족·revert 시
xgram payment mark-failed \
  --id <PAYMENT_ID> \
  --reason "out of gas"
```

## 기대 결과

```
$ xgram payment chains
base    USDC    0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913

$ xgram payment new --amount 25.00 --chain base --to 0x...dEaD --memo "..."
✓ payment intent 생성
  id      : 01HZ9X...
  amount  : 25.000000 USDC (25000000 base units)
  chain   : base
  to      : 0x000000000000000000000000000000000000dEaD
  state   : draft
  created : 2026-05-04 14:21:08+09:00

$ xgram payment sign --id 01HZ9X...
✓ master ECDSA 서명 완료 — state=signed

$ xgram payment mark-confirmed --id 01HZ9X...
✓ state=confirmed (tx 0xabc...)
```

## 주의점

- **fallback 금지**: `payment sign` 의 password 누락은 raise. 환경변수 미설정 시 기본 password 추정 같은 fallback 절대 금지.
- **롤백 가능**: `mark-submitted` 까지는 DB 상태만 바뀌므로 reset 가능. `mark-confirmed` 이후에는 chain 상 트랜잭션이 이미 확정되었으므로 OpenXgram 측 롤백은 의미 없음 — 이전에 외부 broadcast 단계에서 신중히 검토.
- **DB 변경 승인**: `payment new/sign/mark-*` 는 자기 결제 intent 한정 — 자동 가능. 다만 `sign` 은 keystore password 소유 = 마스터 승인과 동치.
- **KST 시간대**: 모든 created_at, signed_at, submitted_at, confirmed_at 은 Asia/Seoul.
- 큰 금액(예: 100 USDC 초과) 자동 정책은 후속 PR — 현재는 매 sign 마다 password 입력으로 sufficient MFA.

## 관련 PRD

- `docs/prd/PRD-OpenXgram-v1.md` Payment 섹션 (USDC on Base)
- `docs/prd/PRD-OpenXgram-v1.md` ECDSA 서명 정책
