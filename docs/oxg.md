# OpenXgram LLM 가이드 (oxg.md)

이 파일은 OpenXgram 이 설치된 환경의 모든 LLM 에이전트가 읽어야 하는
오케스트레이션·통신·확장 가이드입니다. 전역 CLAUDE.md / .cursorrules /
AGENTS.md 등에서 `@~/oxg.md` 로 reference 하세요.

설치 직후 `~/oxg.md` 위치에 자동 복사됩니다 (install.sh).

---

## 0. 세션 시작 시 (권장)

```text
1) openxgram.whoami            ← 내 alias·address·data_dir 확인
2) openxgram.recv_messages(limit=5) ← 받은 메시지 (다른 peer / 사용자 inbound)
```

이 두 호출은 토큰 cost 적고 (각각 50-200 토큰) 전체 작업 흐름을 잡아 줍니다.

---

## 1. Peer 통신 (다른 에이전트와 소통)

### 누가 있는가
```text
openxgram.list_peers           → [{alias, address, last_seen}, ...]
```

### 보내기
```text
openxgram.peer_send(alias, body)
  alias: list_peers 의 alias 사용
  body:  자연어 텍스트 또는 구조화 JSON
```

### 받기 (답장 또는 외부 메시지)
```text
openxgram.recv_messages(limit?, since_rfc3339?, sender?)
  - limit: 기본 10
  - sender: 특정 alias 의 메시지만
  - since_rfc3339: 그 시각 이후만 (예: "2026-05-25T01:00:00+09:00")
```

### 과거 대화 회상 (벡터 검색)
```text
openxgram.recall_messages(query, k?)
  - sqlite-vec KNN 으로 자연어 검색
  - 옛 결정 / 기술 논의 / 사용자 요청 등 찾을 때 사용
```

---

## 2. 외부 채널 발신 (Discord / Telegram)

**agent-push 패턴** — 사용자 의도가 명확할 때 (예: "Discord 로 보내줘",
"이 결과를 채널에 공유해줘") 만 호출. 자동 echo 안 합니다.

### Discord
```text
openxgram.send_to_discord(content, channel?, bot_id?)
  - content: 보낼 텍스트
  - channel: Discord channel_id (e.g. "1505791143307247678")
             생략 시 webhook 모드 fallback (vault notify.discord.webhook_url)
  - bot_id:  여러 봇 등록되어 있을 때 명시 (discord_bots.id).
             생략 시 첫 active 봇 자동 사용.
```

### Telegram
```text
openxgram.send_to_telegram(content, chat_id?)
  - content: 보낼 텍스트
  - chat_id: Telegram chat_id (생략 시 vault notify.telegram.chat_id 사용)
```

### 받기 (inbound)
Discord/Telegram 메시지가 봇 채널로 오면 자동으로 바인딩된 터미널 세션에
주입됩니다. 형식:
```
[Discord:username]
본문 줄1
본문 줄2
```

이 prefix line 은 outbound 와 구별되니 사용자에게 inbound 임을 즉시 인식할 수
있습니다.

---

## 3. Vault (자격증명)

```text
XGRAM_KEYSTORE_PASSWORD env 가 설정되어 있을 때만 사용.
vault_get(key)            → bytes
vault_set(key, bytes)     → ()
vault_list                → [key, ...]
```

다른 에이전트의 vault 는 접근 불가 (ACL).

---

## 4. L2 메모리 (사실·결정·규칙)

```text
openxgram.list_memories_by_kind(kind)
  kind: 'fact' | 'decision' | 'reference' | 'rule'
```

`recall_messages` 가 L0 (원시 메시지) 라면 L2 는 정제된 사실 / 결정.

---

## 5. 오케스트레이션 패턴

### 5.1 단순 위임
```text
1) peer_send(target, "...작업 요청...")
2) 30초~수분 대기 또는 사용자 직접 ask
3) recv_messages(sender=target)
4) 결과 정리
```

### 5.2 병렬 fan-out
```text
1) [peer_send(A, ...), peer_send(B, ...), peer_send(C, ...)] 동시
2) 30초~1분 대기
3) recv_messages(limit=20, since_rfc3339=...) 일괄 수집
4) 답변 merge / 비교 / 결정
```

### 5.3 외부 공유 (Discord)
```text
1) 사용자에게 답변 출력
2) "Discord 로 공유해드릴까요?" 확인 후 (또는 사용자가 명시 요청 시)
   openxgram.send_to_discord(content, channel=<현재 binding channel>)
```

---

## 6. 절대 규칙 (위반 금지)

1. **다른 peer 의 vault 접근 금지** — ACL 위반
2. **채널 입장 메시지에 응답 금지** — 작업 요청 (type=request) 만 답
3. **Discord/Telegram 발신은 사용자 의도 명시 후만** — 자동 echo 안 함
4. **메시지에 비밀번호·토큰 평문 금지** — vault 사용
5. **`openxgram` 가 아닌 임의 도구로 peer 와 직접 통신 시도 금지** — 신원·서명 깨짐

---

## 7. 환경 호환성

이 가이드는 다음 환경에서 동일하게 작동:
- Claude Code (이 가이드 + Skill 시스템 보조)
- Cursor / Continue / Aider / Cline (이 가이드 + MCP 도구)
- Gemini CLI (이 가이드 + MCP)
- 그 외 MCP 호환 환경

Skill 시스템은 Claude Code 전용 — 핵심 정보는 이 파일 + MCP 도구로 완결되도록
설계되어 있습니다.

---

## 8. 디버그 / 확장

세부 흐름이 의심스러우면:
- `journalctl --user -u openxgram-sidecar -f` (Linux)
- DB 직접: `sqlite3 ~/.openxgram/db.sqlite "SELECT ..."` (읽기 전용 권장)
- 새 MCP 도구 추가는 PR 환영 — `crates/openxgram-cli/src/mcp_serve.rs`

---

이 파일은 `xgram update` 또는 OpenXgram 새 버전 install 시 갱신됩니다.
사용자가 직접 수정하면 다음 update 에서 덮어쓸 수 있으니, 개인 메모는
`~/.openxgram/local_notes.md` 같은 별도 파일에 두세요.
