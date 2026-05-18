# 04 — Nostr cross-network 메시지

> 한 줄 요약: 서로 다른 네트워크에 있는 두 머신을 Nostr relay 를 통해 연결하고, peer alias 의 `nostr://` 주소만으로 암호화 메시지를 주고받는다.

## 시나리오

집 NAS(`home-nas`)는 NAT 뒤에 있어 외부에서 직접 HTTP 로 접근할 수 없고, 이동 중인 노트북(`mac-laptop`)도 카페 Wi-Fi 에 있어 IP 가 자주 바뀐다. 두 머신은 동일 master 키를 공유하는 OpenXgram 노드. 자체 호스팅한 Nostr relay (또는 공용 relay) 를 경유해 서로 messages 를 publish/subscribe 한다. peer 의 address 가 `nostr://relay.example.com` 으로 시작하면 자동으로 NostrSink 라우팅 — 같은 `xgram peer send` 로 상대 scheme 만 바뀐다.

## 사전 준비

- 두 머신 모두 `xgram init` 완료, 시드 import 로 동일 master 키 공유
- relay: 자체 호스팅(`xgram relay`) 또는 공용 relay (예: `wss://relay.damus.io`)
- 양쪽 모두 상대방의 secp256k1 pubkey hex 를 알고 있어야 함 (`xgram keypair show --name master`)

## 단계별 명령 시퀀스

```bash
# 1) (옵션) home-nas — 자체 Nostr relay 띄우기
xgram relay --bind 0.0.0.0:7777
# → 다른 머신은 nostr://home-nas-public-host:7777 로 접속

# 2) mac-laptop — home-nas 를 peer 로 등록 (nostr 주소)
xgram peer add \
  --alias home-nas \
  --public-key 02ab...home_nas_pubkey_66hex... \
  --address nostr://relay.example.com:7777 \
  --role primary \
  --notes "home NAS, NAT 뒤"

# 3) home-nas — mac-laptop 을 peer 로 등록
xgram peer add \
  --alias mac-laptop \
  --public-key 03cd...mac_pubkey_66hex... \
  --address nostr://relay.example.com:7777 \
  --role secondary

# 4) mac-laptop — 메시지 전송 (master ECDSA 서명 + Nostr 암호화)
export XGRAM_KEYSTORE_PASSWORD="..."
xgram peer send \
  --alias home-nas \
  --body "ratchet test 1 — 2026-05-04 KST"

# 5) home-nas — Nostr inbound 데몬으로 수신 (sidecar 가 자동 처리)
xgram daemon --tailscale  # 기본 transport
# 또는 별도 worktree 에서 inbound 만 별도 운영 (후속 PR — 현재는 daemon 통합)

# 6) 여러 머신에 동시 전송 — broadcast
xgram peer send  # (단일 alias 만 지원 — 여러 alias 는 broadcast 사용)
xgram peer broadcast \
  --aliases home-nas,gcp-main,laptop \
  --body "5분 뒤 deploy 시작"

# 7) peer last_seen 확인
xgram peer list
xgram peer show home-nas
```

## 기대 결과

```
$ xgram peer send --alias home-nas --body "ratchet test 1"
✓ peer send 완료
  alias       : home-nas
  route       : nostr://relay.example.com:7777
  event_kind  : 4001
  event_id    : 0x9f...
  signed_at   : 2026-05-04 14:33:21+09:00

$ xgram peer broadcast --aliases home-nas,gcp-main --body "..."
✓ broadcast 완료 — 2/2 성공
  ✓ home-nas
  ✓ gcp-main
```

## 주의점

- **fallback 금지**: HTTP 실패 시 자동 Nostr fallback 은 **명시적 opt-in** 필요 — `XGRAM_PEER_FALLBACK_NOSTR=1` + `XGRAM_PEER_FALLBACK_NOSTR_RELAY=wss://...` 두 환경변수 모두 있어야 동작 (ADR-NOSTR-FALLBACK). 둘 중 하나라도 없으면 silent fallback 절대 안 됨 — raise.
- **롤백 가능**: peer 정보는 로컬 DB 만 변경 — `peer delete` 로 즉시 제거. relay 에 publish 된 event 는 외부 — 회수 불가능하므로 송신 전 신중.
- **DB 변경 승인**: `peer add/delete` 는 자기 머신 한정 자동. send/broadcast 는 master 서명 사용 → password 필요.
- **KST 시간대**: peer.last_seen, event signed_at 모두 Asia/Seoul. Nostr event 자체는 unix timestamp 지만 표시는 KST 변환.
- relay 가 다운되면 publish 실패 — 명시적 에러로 raise. 큐에 쌓아 재시도하는 자동 retry 는 후속 PR.

## 관련 PRD

- `docs/prd/PRD-OpenXgram-v1.md` Nostr Transport 섹션
- ADR-NOSTR-FALLBACK (HTTP→Nostr opt-in fallback)
- PRD-NOSTR-03/04 (NostrSink + NostrSource), PRD-NOSTR-06 (self-host relay), PRD-NOSTR-07 (peer scheme 라우팅)
