# 현황 그리드 peer session_identifier·status 양쪽 동기화 — rc.370 구현 계획

> 목표: seoul·zalman 현황 그리드에서 **kind=peer 행의 (alias, session_type, status) 셋이 양쪽 완전 동일**.
> 현재 결함: 원격-홈 peer(예 codex-ai-image@zalman)의 session_identifier 가 머신마다 다름.
> 정본 버전: rc.369 → **rc.370 bump**.

---

## 1. 확정 진단 (실측 — 추측 아님)

### 1.1 seoul DB 실측 (`/home/llm/.openxgram/db.sqlite`, peers 테이블)
- `codex-ai-image` (eth `0x5E12A7C8...`): seoul = **`session_identifier=acp:acp-1`, `session_status=active`** ← 날조값.
- `teeup` (eth `0xD8c6bcEC...`): seoul = **`acp:acp-1`, `active`** ← 동일 날조.
- 진짜(홈=zalman) 값은 `tmux:aoe_codex-ai-image_5322fc47` 여야 함.
- 스키마: peers 컬럼은 `session_status`(NOT `status`), `session_identifier`. (kv 테이블 없음.)

### 1.2 acp:acp-1 가 어떻게 seoul DB 에 쓰이는가 (근본 원인 #A — LOCAL 발명)
파일: `crates/openxgram-cli/src/daemon_gui_acp.rs`
- `bridge_session_as_peer(session_id, label, agent)` (line ~192) 가:
  ```
  UPDATE peers SET session_identifier = 'acp:<sid>', session_status = 'active' WHERE alias = ?
  ```
  (line ~224) 를 **alias 만으로** 실행 — 그 alias 가 **원격-홈 peer 인지 전혀 확인 안 함**.
- 호출 지점: daemon_gui_acp.rs line 654, **774** (`bridge_session_as_peer(session_id, conv_key.as_deref(), &bridge_agent)`).
- 트리거 경로: `crates/openxgram-cli/src/daemon.rs` process_inbound (line ~1424) `is_acp_drivable(&mut db, &target_alias)`:
  ```
  SELECT 1 FROM agent_capabilities WHERE alias = ? AND role IS NOT 'tmux' LIMIT 1
  ```
  → codex-ai-image/teeup 가 `agent_capabilities` 에 role≠'tmux' 로 있으면 true →
  seoul 이 **로컬 ACP 세션(acp-1)을 spawn → bridge → peers row 를 acp:acp-1 로 덮어씀.**
- `new_session_id()` (daemon_gui_acp.rs) = `format!("acp-{n}")` → 첫 세션이 `acp-1` → `acp:acp-1`.
- `map_session_to_peer_upsert` (acp_peer_bridge.rs line ~89): label 만 보고 PeerUpsert 생성, **원격 여부 게이트 없음.**

### 1.3 왜 영영 고쳐지지 않는가 (근본 원인 #B — gossip 갭)
파일: `crates/openxgram-cli/src/daemon_peer_sync.rs`
- DTO `RemotePeer` (line ~42): `alias, public_key_hex, eth_address, address, gui_address, role, display_name`.
  → **session_identifier·session_status 필드 자체가 없음.** gossip 이 신원만 나르고 세션상태는 안 나름.
- `reachable_remote_peers` (line ~307): 광고 시 `display_name` 은 실음(name_map), **session_identifier·status 안 실음.**
- `merge_remote_peers` (line ~80): role/display_name 만 `update_identity_fields` 로 권위 갱신,
  **session_identifier·session_status 는 절대 안 받음.** → 홈(zalman)의 진짜 tmux 값이 seoul 로 못 옴.

### 1.4 status 도출 (참고 — 정상이나 입력이 오염됨)
파일: `crates/openxgram-peer/src/identity.rs` `roster_from_sources` (line ~396, status 계산 ~525-550):
- `tmux:` sid + live집합에 있음 → active / 없음 → dead
- 비-tmux sid(acp:) 또는 sid 없음 → stopped (또는 session_status 반영)
- ⚠️ 입력 session_identifier 가 오염(acp:acp-1)되면 도출도 오염. → #A·#B 고치면 자동 정상.

---

## 2. 수정 (rc.370) — 두 갈래

### 2.1 #A 차단 — ACP 브리지가 원격-홈 peer 를 덮어쓰지 못하게 (소유권 게이트)
파일: `daemon_gui_acp.rs` `bridge_session_as_peer`
- peers UPDATE(line ~224) **직전**에 가드 추가: 그 alias 의 기존 peers row 가
  **원격-홈**(address host ≠ self_host) 이면 session_identifier·status UPDATE 를 **skip** (명시 로그, 절대규칙1 — silent X).
- self_host 판정은 daemon_peer_sync.rs 와 동일 우선순위:
  `XGRAM_TRANSPORT_PUBLIC_URL` → `XGRAM_SELF_ADDRESS` → self peer row(eth==self_eth) → url_host().
  → 공통 헬퍼로 추출 권장 (`daemon_peer_sync::self_machine_host(data_dir)` 같은 pub fn 신설, 중복 금지).
- 추가 방어 (권장): `is_acp_drivable` 또는 inbound 라우팅이 **원격-홈 peer** 면 로컬 ACP spawn 자체를 안 하고
  홈 머신으로 A2A 라우팅(transport envelope)하도록. 최소 구현은 위 UPDATE skip 만으로 그리드 표시는 일치됨.
- ⚠️ 회귀 방지: 진짜 LOCAL ACP 에이전트(self_host 에 홈된, address 없거나 self host)는 종전대로 브리지돼야 함.

### 2.2 #B 보강 — gossip 이 홈의 session_identifier·status 를 권위 전파
파일: `daemon_peer_sync.rs`
1. DTO `RemotePeer` 에 필드 2개 추가 (serde default + skip_if None, 하위호환):
   ```rust
   #[serde(default, skip_serializing_if = "Option::is_none")]
   pub session_identifier: Option<String>,
   #[serde(default, skip_serializing_if = "Option::is_none")]
   pub session_status: Option<String>,
   ```
2. `reachable_remote_peers` — 광고 시 sid_map 에서 가져온 session_identifier 를 실어보냄.
   status_map(`SELECT alias, session_status FROM peers WHERE session_status IS NOT NULL AND session_status<>''`)
   prefetch 추가(name_map 패턴 동일) → session_status 실음.
   ⚠️ **자기 머신이 홈인 peer 만 광고** (이미 reachable_remote_peers 가 self_host 게이트로 원격 제외) →
   광고된 sid 는 홈 권위값. (seoul 이 codex-ai-image 를 광고 안 함 → 정상.)
3. `merge_remote_peers` — upsert_announce 후, role/display_name 갱신과 **같은 자리에서**
   원격이 보낸 session_identifier·session_status 를 그 peer 의 홈 권위값으로 UPDATE:
   ```
   UPDATE peers SET session_identifier = COALESCE(?,session_identifier),
                    session_status = COALESCE(?,session_status) WHERE eth_address = ? (또는 alias)
   ```
   → zalman 이 광고한 `tmux:aoe_codex-ai-image_5322fc47` / status 가 seoul peers 에 수렴.
   ⚠️ COALESCE 로 None 이 기존값 안 지우게. (홈은 항상 실어주므로 보통 Some.)

### 2.3 기존 오염 데이터 1회 교정 (마이그레이션 아님 — 배포 시 SQL)
seoul DB 의 codex-ai-image·teeup 등 **원격-홈인데 acp:acp-1 인 row** 를 NULL 로 리셋
(다음 gossip tick 이 홈 값으로 채움). 백업 필수.
```
sqlite3 db.sqlite "UPDATE peers SET session_identifier=NULL, session_status=NULL
  WHERE session_identifier='acp:acp-1'"
```
(⚠️ aoe_starian-portal_e145c0d4 = 마스터 라이브, 비개입. 백업·rollback 기록.)

---

## 3. 빌드·배포 (rc.347 소유권격리·cruft차단 유지)

1. version.json `0.2.0-rc.369` → `0.2.0-rc.370` + Cargo workspace version (둘 다 — gotcha).
2. 빌드(seoul): `CARGO_TARGET_DIR=/tmp/xgram-fix /home/llm/.cargo/bin/cargo build --release -p openxgram-cli` (~15분, full path).
   실제 exit code + strings 로 검증 (`| tail` 금지).
3. seoul 배포: 백업 → mv 바이너리 `/home/llm/.local/bin/xgram` → `systemctl --user restart openxgram-sidecar` → `/v1/auth/unlock` 200 확인. rollback 경로 기록.
   프론트 변경 시 dist → `/home/llm/.local/share/openxgram-gui/` cp.
4. zalman 배포: 스크립트파일 방식(cat>/tmp/x.sh<<'EOF'…EOF → scp → cp /mnt/c/.../x.sh ~/x.sh && bash ~/x.sh; % 금지, grep 스크립트 안).
   바이너리 `/home/pasia/.local/bin/xgram`, mv·SIGKILL-9·동기실행·백업·health·rollback. **레거시 17400 부활 금지.**
   접속: `ssh -o ConnectTimeout=8 zalman 'wsl -- bash -lc "..."'`, pw `sd4132sd1234`, db `/home/pasia/.xgram/bots/hermes-z/db.sqlite`, gui 47301.

---

## 4. 검증 (완료 시만 "됐다" — 추측 금지)

1. codex-ai-image session_identifier 양쪽 동일 (zalman=tmux:… → seoul 도 tmux:… 참조, acp:acp-1 사라짐).
2. status 양쪽 동일 (원격 active → seoul 도 active).
3. 양쪽 그리드 roster 에서 kind=peer 행 (alias, session_type, status) 셋 일치 실측 (DB SELECT + GUI /v1/gui/... 둘 다).
4. hermes/codex 독립 검증 (다른 LLM — Claude 자가검증 금지).
5. 변경/배포/rollback 내역 + 안 되면 정확한 막힌 지점.

---

## 5. 핵심 코드 지점 요약 (빌더용)

- 발명: `daemon_gui_acp.rs:224` UPDATE (alias-only, 원격 게이트 없음) ← #A
- 트리거: `daemon.rs:1424` is_acp_drivable → spawn → bridge (line 774 호출)
- DTO 갭: `daemon_peer_sync.rs:42` RemotePeer (sid/status 필드 없음) ← #B
- 광고: `daemon_peer_sync.rs:307` reachable_remote_peers (sid/status 안 실음)
- 병합: `daemon_peer_sync.rs:80` merge_remote_peers (sid/status 안 받음)
- 도출(정상): `identity.rs:396` roster_from_sources (입력 오염이 원인)
- self_host 우선순위: `daemon_peer_sync.rs:376-399`
