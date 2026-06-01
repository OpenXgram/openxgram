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

### 누가 있는가 (동적 lookup)
```text
openxgram.list_peers
→ [{alias, address, role, description, capabilities, eth_address, ...}, ...]
```
정적 list 가 어디에도 박혀있지 않음 — 매 세션 이 호출로 실시간 조회.
새 peer 가 join 하면 즉시 보임 (별도 inject/refresh 명령 불필요).

각 peer 의 `role` / `description` / `capabilities` 는 그 peer 가 직접 등록
(`register_subagent` MCP 또는 `xgram peer register`). 따라서 어떤 머신/환경
에서든 일관된 정보가 보임 — 진정한 분산 신원.

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

### 자동 echo 룰 (중요)
`[Discord:user]` 또는 `[Telegram:user]` prefix 가 포함된 inbound 메시지를
받으면 **답변을 작성한 직후 동일 채널로 send_to_discord/send_to_telegram
자동 호출**. 사용자가 Discord/Telegram 을 메인 UI 로 사용하기 위함.

흐름:
1. 사용자가 Discord 채널에서 메시지 → terminal 에 `[Discord:user] ➤ 본문` 도착
2. LLM 이 답변 작성
3. **자동으로** `openxgram.send_to_discord(content=답변, channel=<inbound 채널 id>)`
4. 사용자가 Discord 에서 답 확인 (터미널 확인 불필요)

채널 id 는 binding (`session_channel_bindings.channel_ref`) 에서 가져옴 — 
첫 active discord binding 자동.

### Discord 카드 형식 (rc.131 ~ 필수)

**plain text 대신 embed 사용**. 매번 사용자가 요청하지 않아도 기본값.

표준 embed 형식:
```json
{
  "embeds": [{
    "title": "...",
    "description": "...",
    "color": 2326507,
    "fields": [
      {"name": "...", "value": "...", "inline": false}
    ],
    "footer": {"text": "..."},
    "timestamp": "<ISO 8601 UTC>"
  }]
}
```

색상 코드 (10진 정수):
- 성공: `2326507` (0x238636 GitHub green)
- 경고: `13801506` (0xD29922 amber)
- 실패: `16270153` (0xF85149 red)
- 정보: `3823210` (0x3A4A6A blue-grey)

마크다운 사용:
- `**bold**` / `*italic*` / `` `inline code` ``
- ` ```multi-line code``` `
- `> quote`
- `# ## ###` headers
- `-` 리스트 / `1.` 번호 리스트
- `[text](url)` 링크
- 이모지 / Unicode 문자 자유

길이 분기:
- 한두 문장 짧은 답 → content 만 + 마크다운
- 작업 완료 보고 / 진단 / plan / 검증 결과 → embed 사용
- 매우 긴 출력 (>2000자) → `.txt` 첨부 (multipart)

호출 예 (`openxgram.send_to_discord` 가 embeds field 미지원 시 직접 HTTP):
```bash
curl -X POST "https://discord.com/api/v10/channels/{channel_id}/messages" \
  -H "Authorization: Bot ${BOT_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"embeds":[{...}]}'
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

### 5.0 role 기반 위임 (권장)
```text
1) openxgram.list_peers  → 각 peer 의 role/description 확인
2) 사용자 의도 → role 매핑 (예: "PRD 갱신" → role="prd-author" 또는 description 매칭)
3) 매칭된 peer alias 로 peer_send
4) 답 대기 → recv_messages

예시 매핑 (OpenXgram 의 표준 role, 실제로는 list_peers 결과 우선):
- "PRD 작성·갱신"  → Pip
- "Rust 코어 구현"  → Eno
- "테스트·검증"     → Qua
- "외부 라이브러리 리서치" → Res
```
role 은 자유 텍스트. 자연어 의미 매칭으로 충분.

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
6. **🔥 자율 해결 우선 — 자체 시스템 이슈는 peer 메신저로 먼저 (강제)**

   OpenXgram·MCP·daemon·vault·keystore·peer·tmux·Discord auto-echo 등 **자체 시스템 이슈** 발생 시 다음 순서로 처리:

   - **1단계 (필수)**: 같은 시스템 사용 중인 다른 peer 에게 `peer_send` 로 진단·해결책 요청.
     - 예: `peer_send(alias="Starian", body="내 MCP 의 peer_send 가 -32602 fail. vault password env 부재 추정. 너의 환경에서는? 해결 path 추천?")`
   - **2단계**: peer 의 답신 + 자기 환경 진단 결과 종합 → 시도.
   - **3단계 (마지막)**: 위 두 단계 모두 fail 시에만 마스터에게 명확한 상황 + 결정 요청.

   **금지 안티패턴**:
   - "마스터, env 박아주세요" 같은 1차 의존 요청 X
   - peer 통신 없이 추측만으로 마스터에게 보고 X
   - 마스터 = 최종 결정권자, 1차 troubleshooter 가 아님.

   **정당 형식** (마스터 호출 시):
   - "Starian peer 에게 진단 받았고 X·Y 시도. Z 결과 → 마스터 결정 필요 (A: 빠른 우회 / B: 본질 fix)?"

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
