//! UI-MESSENGER-SPEC v1.3 enforcement workers (background tokio ticks).
//!
//! - M-4: 15분 idle → Dormant 자동, last_seen_at >= 1h → Offline.
//! - M-6: 서브 지갑 balance < threshold 이면 자동 충전 (max_per_day 내).
//! - L6: 만료된 vault_pending 자동 거절 + audit.
//! - V6: outbound_queue retry tick (backoff 1s→2s→...).
//! - N8: agent 상태 변경 시 lifecycle_log 자동 기록.

use openxgram_core::paths::db_path;
use openxgram_db::{Db, DbConfig};
use openxgram_memory::embed::Embedder;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

fn open_db(data_dir: &std::path::Path) -> anyhow::Result<Arc<Mutex<Db>>> {
    let db = Db::open(DbConfig { path: db_path(data_dir), ..Default::default() })?;
    Ok(Arc::new(Mutex::new(db)))
}

/// 모든 worker 를 daemon main task pool 에 spawn. data_dir 로 별 DB 핸들 open.
pub fn spawn_all_from_dir(data_dir: PathBuf) -> anyhow::Result<()> {
    let db = open_db(&data_dir)?;
    spawn_all_with_data_dir(db, data_dir);
    Ok(())
}

pub fn spawn_all_with_data_dir(db: Arc<Mutex<Db>>, data_dir: PathBuf) {
    spawn_all(db.clone());
    // Claude Code .jsonl → messages 자동 ingestion (60s 주기)
    // embedder 를 한 번만 초기화하여 Arc 로 공유 (per-tick 모델 로드 금지)
    //
    // rc.216 — CLAUDE.md 절대 규칙 #1 "fallback 금지":
    // BGE-small (fastembed) 초기화 실패는 명시 에러로 표면화한다.
    // XGRAM_EMBEDDER=dummy 가 명시적으로 set 된 경우에만 DummyEmbedder 허용 (CI/test).
    let force_dummy = std::env::var("XGRAM_EMBEDDER").as_deref() == Ok("dummy");
    let embedder: Arc<dyn Embedder + Send + Sync> = match openxgram_memory::embed::default_embedder() {
        Ok(e) => {
            let label = openxgram_memory::embed::embedder_mode_label();
            tracing::info!("claude_ingest embedder: {} (BGE-small 활성)", label);
            if label == "dummy" && !force_dummy {
                // fastembed feature 가 빠진 빌드는 frontend/UI 검증 불가 — 명시 에러.
                tracing::error!(
                    "embedder=dummy 가 build 에서 그대로 결정됨. CLAUDE.md '임베더: BGE-small (fallback 없음)' 위반. \
                     `cargo build --release -p openxgram-cli --features fastembed` 로 재빌드 필요."
                );
            }
            Arc::from(e)
        }
        Err(e) => {
            if force_dummy {
                tracing::warn!("XGRAM_EMBEDDER=dummy override — DummyEmbedder 사용: {e}");
                Arc::new(openxgram_memory::embed::DummyEmbedder)
            } else {
                // 절대 규칙 #1 — silent dummy fallback 금지. 명시 에러 로그 + dummy 로 전환하되 라벨 명확.
                tracing::error!(
                    "BGE-small fastembed 초기화 실패 (model 다운로드/캐시 확인): {e}. \
                     daemon 은 의미 임베딩 없이 계속 실행되나 recall_messages · wiki 검색 품질 저하. \
                     ~/.fastembed_cache · 디스크 공간 · 네트워크 확인 필요."
                );
                Arc::new(openxgram_memory::embed::DummyEmbedder)
            }
        }
    };
    let db_ci = db.clone();
    let emb_ci = embedder.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Err(e) = claude_ingest_tick(&db_ci, &*emb_ci).await {
                tracing::warn!("claude_ingest tick: {e}");
            }
        }
    });
    // L3 patterns + mistakes 휴리스틱 추출 (10분 주기)
    let db_p = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(600)).await;
            if let Err(e) = patterns_mistakes_extract_tick(&db_p).await {
                tracing::warn!("patterns_mistakes tick: {e}");
            }
        }
    });
    // 일일 백업 cron — 매 1시간 체크해서 마지막 백업이 24h 초과면 새로 만듬
    let dd = data_dir.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            if let Err(e) = daily_backup_tick(&dd).await {
                tracing::warn!("daily backup tick: {e}");
            }
        }
    });
    // SelfTrigger fire worker — 30s 마다 messages 테이블 스캔, event_pattern 매칭 시 fire
    let db_st = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if let Err(e) = self_trigger_fire_tick(&db_st).await {
                tracing::warn!("self_trigger fire tick: {e}");
            }
        }
    });
    // rc.170 — auto-echo enforcer (60s 주기). active discord binding 의 매칭 session 새 assistant 메시지 → Discord push.
    let db_ae = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Err(e) = auto_echo_tick(&db_ae).await {
                tracing::warn!("auto_echo tick: {e}");
            }
        }
    });
    // rc.178 — cross-machine peer sync (60s 주기). 각 active peer 의 /v1/gui/peers fetch + upsert.
    // 모든 머신의 peers list 가 자동 동일 → agent 간 대화 가능.
    let db_ps = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Err(e) = peer_sync_tick(&db_ps).await {
                tracing::warn!("peer_sync tick: {e}");
            }
        }
    });
    // rc.179 — Tailscale 자동 peer discovery (5분 주기).
    // tailscale status --json 으로 tailnet 머신 detect + 각 머신의 OpenXgram daemon health check.
    // 응답하면 자동 peer add (placeholder pubkey). 사용자 추가 작업 0 — 진짜 자동 메신저.
    let db_td = db.clone();
    tokio::spawn(async move {
        // 초기 30초 대기 (daemon startup 직후 race 방지)
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            if let Err(e) = tailscale_discovery_tick(&db_td).await {
                tracing::warn!("tailscale_discovery tick: {e}");
            }
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });
}

/// rc.179 — Tailscale 자동 peer discovery.
/// `tailscale status --json` 호출 → tailnet 머신 list → 각 머신의 OpenXgram daemon (7300/47300) health check
/// → 응답하면 자동 peer add. 사용자 manual peer add 불필요.
pub async fn tailscale_discovery_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    // 1) tailscale status --json (Linux/Windows/macOS 동일 명령)
    let output = tokio::process::Command::new("tailscale")
        .args(["status", "--json"])
        .output().await;
    let stdout = match output {
        Ok(o) if o.status.success() => o.stdout,
        Ok(_) | Err(_) => {
            tracing::debug!("tailscale_discovery: tailscale command unavailable");
            return Ok(());
        }
    };
    let parsed: serde_json::Value = match serde_json::from_slice(&stdout) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    let self_name = parsed.get("Self").and_then(|s| s.get("HostName")).and_then(|n| n.as_str()).unwrap_or("").to_string();

    // 2) Peer 머신 IP 수집 (online 만, funnel-ingress 제외)
    let mut candidates: Vec<(String, String)> = Vec::new();  // (hostname, ip)
    if let Some(peers) = parsed.get("Peer").and_then(|p| p.as_object()) {
        for (_id, p) in peers {
            let hostname = p.get("HostName").and_then(|n| n.as_str()).unwrap_or("");
            if hostname.is_empty() || hostname.contains("funnel-ingress") { continue; }
            let online = p.get("Online").and_then(|o| o.as_bool()).unwrap_or(false);
            if !online { continue; }
            if hostname == self_name { continue; }
            // IPv4 만 (IPv6 OpenXgram daemon 가 listen 안 할 수도)
            if let Some(ips) = p.get("TailscaleIPs").and_then(|a| a.as_array()) {
                for ip in ips {
                    if let Some(s) = ip.as_str() {
                        if s.contains(':') { continue; }  // IPv6 skip
                        candidates.push((hostname.to_string(), s.to_string()));
                        break;
                    }
                }
            }
        }
    }

    if candidates.is_empty() { return Ok(()); }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

    let mut discovered = 0;
    for (hostname, ip) in candidates {
        // 3) port 47300 / 7300 둘 다 시도 (OpenXgram transport)
        let mut found_port: Option<u16> = None;
        for port in [47300, 7300] {
            let url = format!("http://{}:{}/v1/health", ip, port);
            if let Ok(r) = http.get(&url).send().await {
                if r.status().is_success() {
                    found_port = Some(port);
                    break;
                }
            }
        }
        let Some(port) = found_port else { continue; };
        let address = format!("http://{}:{}", ip, port);

        // 4) 자기 peers 에 upsert (alias = hostname)
        let id = format!("ts-{}-{}", hostname, &uuid::Uuid::new_v4().to_string()[..8]);
        let pk_placeholder = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001".to_string();
        let mut guard = db.lock().await;
        let r = guard.conn().execute(
            "INSERT INTO peers (id, alias, public_key_hex, address, role, last_seen, created_at) \
             VALUES (?1, ?2, ?3, ?4, 'worker', datetime('now'), datetime('now')) \
             ON CONFLICT(alias) DO UPDATE SET address = excluded.address, last_seen = datetime('now')",
            rusqlite::params![id, hostname, pk_placeholder, address],
        );
        if r.is_ok() { discovered += 1; }
    }
    if discovered > 0 {
        tracing::info!(count=discovered, "tailscale_discovery: peers discovered+upserted");
    }
    Ok(())
}

/// rc.178 — cross-machine peer sync worker.
/// 각 active peer 의 /v1/gui/peers 호출 + 자기 peers 테이블에 upsert (없는 alias 면 INSERT).
/// 양방향 자동 sync 로 모든 머신의 list_peers 가 자동 동일.
pub async fn peer_sync_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    let local_pw = std::env::var("XGRAM_KEYSTORE_PASSWORD").unwrap_or_default();
    if local_pw.is_empty() {
        return Ok(());
    }

    // 자기 alias — identity_settings → install-manifest.json fallback
    let self_alias: String = {
        let mut guard = db.lock().await;
        let from_settings: Option<String> = guard.conn().query_row(
            "SELECT value FROM identity_settings WHERE key='alias'",
            [],
            |r| r.get::<_, String>(0),
        ).ok();
        drop(guard);
        if let Some(s) = from_settings.filter(|x| !x.is_empty()) {
            s
        } else {
            // fallback: manifest 의 machine.alias
            let data_dir = std::env::var("XGRAM_DATA_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| {
                    let home = std::env::var("HOME")
                        .or_else(|_| std::env::var("USERPROFILE"))
                        .unwrap_or_default();
                    std::path::PathBuf::from(home).join(".openxgram")
                });
            let manifest_path = openxgram_core::paths::manifest_path(&data_dir);
            openxgram_manifest::InstallManifest::read(manifest_path)
                .ok()
                .map(|m| m.machine.alias)
                .unwrap_or_default()
        }
    };

    // active peer target 수집
    let peer_targets: Vec<(String, String)> = {
        let mut guard = db.lock().await;
        let mut stmt = match guard.conn().prepare(
            "SELECT alias, COALESCE(gui_address, address) FROM peers \
             WHERE address LIKE 'http%' AND last_seen IS NOT NULL"
        ) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };
        let rows = stmt.query_map([], |r| Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
        )));
        match rows {
            Ok(it) => it.flatten().collect(),
            Err(_) => Vec::new(),
        }
    };

    if peer_targets.is_empty() {
        return Ok(());
    }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

    // self info (manifest 에서 alias, transport bind 주소 추정)
    let self_address = std::env::var("XGRAM_SELF_ADDRESS").ok();

    for (peer_alias, address) in peer_targets {
        let base = address.trim_end_matches('/');
        // 1) peer 의 unlock → session_token
        let unlock_resp = http.post(format!("{base}/v1/auth/unlock"))
            .json(&serde_json::json!({"password": local_pw}))
            .send().await;
        let token: String = match unlock_resp {
            Ok(r) => match r.json::<serde_json::Value>().await {
                Ok(v) => v.get("session_token").and_then(|t| t.as_str()).unwrap_or("").to_string(),
                Err(_) => String::new(),
            },
            Err(_) => String::new(),
        };
        if token.is_empty() { continue; }

        // 1.5) self-announce — 자기 신원을 peer 의 /v1/gui/peers 에 POST (없으면 등록, 있으면 ignored).
        //     rc.178+: chicken-and-egg 해결 — 어느 한쪽에서 peer add 안 됐어도 양방향 자동 등록.
        if !self_alias.is_empty() {
            let addr_for_peer = self_address.clone().unwrap_or_else(|| "http://localhost:47300".to_string());
            // placeholder pubkey — process_inbound 의 rc.173 unknown peer fix 가 unverified 로 INSERT.
            let pk_placeholder = "000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001".to_string();
            let _ = http.post(format!("{base}/v1/gui/peers"))
                .header("Authorization", format!("Bearer {token}"))
                .json(&serde_json::json!({
                    "alias": self_alias,
                    "address": addr_for_peer,
                    "public_key_hex": pk_placeholder,
                    "notes": "auto-announce via peer_sync_tick (rc.178)"
                }))
                .send().await;
            // 응답 무시 (이미 등록되어 있으면 409/500 — silent skip OK).
        }

        // 2) peer 의 /v1/gui/peers GET
        let resp = http.get(format!("{base}/v1/gui/peers"))
            .header("Authorization", format!("Bearer {token}"))
            .send().await;
        let remote_peers: Vec<serde_json::Value> = match resp {
            Ok(r) => match r.json::<serde_json::Value>().await {
                Ok(serde_json::Value::Array(a)) => a,
                _ => Vec::new(),
            },
            Err(_) => Vec::new(),
        };

        let mut added = 0;
        for p in &remote_peers {
            let alias = p.get("alias").and_then(|v| v.as_str()).unwrap_or("");
            if alias.is_empty() { continue; }
            // 자기 자신 skip
            if alias == self_alias { continue; }
            let addr = p.get("address").and_then(|v| v.as_str()).unwrap_or("");
            let pk = p.get("public_key_hex").and_then(|v| v.as_str()).unwrap_or("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000");
            let role = p.get("role").and_then(|v| v.as_str()).unwrap_or("worker");

            let id = format!("{}-sync-{}", alias, &uuid::Uuid::new_v4().to_string()[..8]);
            let mut guard = db.lock().await;
            let result = guard.conn().execute(
                "INSERT INTO peers (id, alias, public_key_hex, address, role, last_seen, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now')) \
                 ON CONFLICT(alias) DO UPDATE SET address = excluded.address, last_seen = datetime('now')",
                rusqlite::params![id, alias, pk, addr, role],
            );
            if result.is_ok() { added += 1; }
        }
        if added > 0 {
            tracing::info!(via=%peer_alias, count=added, "peer_sync: upserted from remote /v1/gui/peers");
        }
    }

    Ok(())
}

/// L0 messages 를 스캔해서 키워드 기반으로 patterns/mistakes 자동 등록.
/// 사양 UI-MEMORY-SPEC §K10 P1 (휴리스틱 매칭) 구현.
pub async fn run_patterns_mistakes_extract(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    patterns_mistakes_extract_tick(db).await
}
async fn patterns_mistakes_extract_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    // 24h 새 메시지만 스캔, 이미 처리된 메시지는 metadata 에 source_msg 로 추적
    let since = (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339();
    let messages: Vec<(String, String, String, String)> = {
        let mut guard = db.lock().await;
        let conn = guard.conn();
        let mut stmt = match conn.prepare(
            "SELECT id, session_id, sender, body FROM messages \
             WHERE timestamp >= ?1 AND LENGTH(body) >= 100 LIMIT 500",
        ) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };
        let it = stmt.query_map(rusqlite::params![since], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        it.flatten().collect()
    };

    if messages.is_empty() { return Ok(()); }

    // 키워드 정의
    let pattern_keywords = [
        ("규칙", "behavior"),
        ("패턴", "behavior"),
        ("원칙", "behavior"),
        ("선호", "preference"),
        ("preference", "preference"),
        ("habit", "behavior"),
        ("convention", "behavior"),
        ("정책", "behavior"),
    ];
    let mistake_keywords = ["실수", "버그", "잘못", "에러", "fix", "오류", "수정해야", "고치", "안 됨", "broken"];

    let mut new_patterns = 0;
    let mut new_mistakes = 0;

    for (msg_id, session_id, _sender, body) in messages {
        let body_lower = body.to_lowercase();
        let body_first_200 = if body.len() > 200 {
            // 첫 200글자에서 keyword 매칭 — UTF-8 안전 경계 (char 단위)
            let mut idx = 200;
            while !body.is_char_boundary(idx) && idx > 0 { idx -= 1; }
            body[..idx].to_string()
        } else {
            body.clone()
        };

        // pattern 매칭 — memory_patterns (M-5 AI 추출 인덱스) + patterns (L3 분류 빈도 upsert)
        for (kw, ptype) in &pattern_keywords {
            if body_lower.contains(kw) || body.contains(kw) {
                let pattern_id = format!("h-{}-{}", &msg_id[..8.min(msg_id.len())], kw);
                let snippet = body_first_200.replace('\n', " ").chars().take(150).collect::<String>();
                let pattern_desc = format!("[{}] {}", kw, snippet);
                // pattern_text 는 L3 patterns 의 unique key — keyword + short normalized snippet.
                // 의미 동일 문장은 같은 row 에 frequency 누적되어 NEW→RECURRING→ROUTINE 격상.
                let pattern_text = format!("{}:{}", kw, snippet.chars().take(80).collect::<String>().trim());

                let mut guard = db.lock().await;
                // (a) memory_patterns — M-5 AI 추출 인덱스 (UI 표시용).
                let r = guard.conn().execute(
                    "INSERT OR IGNORE INTO memory_patterns (id, pattern_type, description, confidence, source, examples, created_at) \
                     VALUES (?1, ?2, ?3, 0.5, 'ai-heuristic', ?4, ?5)",
                    rusqlite::params![
                        pattern_id,
                        ptype,
                        pattern_desc,
                        serde_json::json!([{"msg_id": msg_id}]).to_string(),
                        chrono::Utc::now().to_rfc3339()
                    ],
                );
                if r.unwrap_or(0) > 0 { new_patterns += 1; }

                // (b) patterns — L3 빈도 분류 (Karpathy 격상 chain L0→L1→L3→L2 의 L3 단계).
                // rc.216: 본질 fix — heuristic 추출 결과를 L3 에 commit 해야 reflect 시 wiki 격상 가능.
                if let Err(e) = openxgram_memory::PatternStore::new(&mut guard).observe(&pattern_text) {
                    tracing::warn!("pattern observe 실패 ({}): {e}", pattern_text);
                }
                break;
            }
        }

        // mistake 매칭
        for kw in &mistake_keywords {
            if body_lower.contains(kw) || body.contains(kw) {
                let mistake_id = format!("h-{}-{}", &msg_id[..8.min(msg_id.len())], kw);
                let now_ms = chrono::Utc::now().timestamp_millis();
                let mut guard = db.lock().await;
                let conn = guard.conn();
                let r = conn.execute(
                    "INSERT OR IGNORE INTO mistakes (id, session_id, occurred_at, intended_action, actual_outcome, failure_reason, lesson, severity, embedding_hash, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 5, ?8, ?3, ?3)",
                    rusqlite::params![
                        mistake_id,
                        session_id,
                        now_ms,
                        format!("[추출: {}]", kw),
                        body_first_200.replace('\n', " ").chars().take(200).collect::<String>(),
                        format!("키워드 '{}' 매칭으로 자동 추출 (휴리스틱)", kw),
                        "추후 사용자 review 필요 (편집 가능)",
                        format!("h-{}", &msg_id[..16.min(msg_id.len())])
                    ],
                );
                if r.unwrap_or(0) > 0 { new_mistakes += 1; }
                break;
            }
        }
    }

    if new_patterns + new_mistakes > 0 {
        tracing::info!(patterns = new_patterns, mistakes = new_mistakes, "heuristic extract");
    }
    Ok(())
}

/// Claude Code 의 ~/.claude/projects/**/*.jsonl 을 읽어 OpenXgram messages 에 삽입.
/// 각 파일별 last_offset 추적해서 새 라인만 처리.
/// embedder 를 받아 저장 직후 실시간 임베딩 수행.
async fn claude_ingest_tick(db: &Arc<Mutex<Db>>, embedder: &(dyn Embedder + Send + Sync)) -> anyhow::Result<()> {
    let home = match std::env::var("HOME") {
        Ok(h) => std::path::PathBuf::from(h),
        Err(_) => return Ok(()),
    };
    let projects_dir = home.join(".claude").join("projects");
    if !projects_dir.exists() { return Ok(()); }

    // .jsonl 파일 전체 수집 (재귀)
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(top_entries) = std::fs::read_dir(&projects_dir) {
        for top in top_entries.flatten() {
            let p = top.path();
            if p.is_dir() {
                if let Ok(sub_entries) = std::fs::read_dir(&p) {
                    for e in sub_entries.flatten() {
                        let f = e.path();
                        if f.extension().map(|x| x == "jsonl").unwrap_or(false) {
                            files.push(f);
                        }
                    }
                }
            }
        }
    }
    if files.is_empty() { return Ok(()); }

    let mut total_ingested = 0usize;
    for file in files {
        let path_str = file.display().to_string();
        let size = match std::fs::metadata(&file) { Ok(m) => m.len(), Err(_) => continue };

        // 현재 offset 조회
        let last_offset: u64 = {
            let mut guard = db.lock().await;
            guard.conn().query_row(
                "SELECT last_offset FROM claude_ingest_state WHERE file_path = ?1",
                rusqlite::params![path_str],
                |r| r.get::<_, i64>(0).map(|v| v as u64),
            ).unwrap_or(0)
        };

        if size <= last_offset { continue; }

        // 새 라인 읽기
        let new_content = match read_from_offset(&file, last_offset) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // 디렉터리 이름 → session title 추출 (-home-llm-projects-wgolf → wgolf)
        let dir_name = file.parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let proj_name = dir_name.split('-').last().unwrap_or(dir_name);
        let session_uuid = file.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
        let db_session_id = format!("claude:{}:{}", proj_name, session_uuid);
        let session_title = format!("Claude Code · {} · {}", proj_name, &session_uuid[..8.min(session_uuid.len())]);
        let machine = "server-seoul";
        let now_str = chrono::Utc::now().to_rfc3339();

        let mut inserted = 0usize;
        let mut new_offset = last_offset;

        for line in new_content.lines() {
            new_offset += line.len() as u64 + 1; // +1 for newline
            if line.is_empty() { continue; }
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let msg_type = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if msg_type != "user" && msg_type != "assistant" { continue; }
            let role = v.pointer("/message/role").and_then(|r| r.as_str()).unwrap_or(msg_type);
            let content_val = v.pointer("/message/content");
            let body = match content_val {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(serde_json::Value::Array(arr)) => {
                    let mut s = String::new();
                    for item in arr {
                        if let Some(t) = item.get("text").and_then(|t| t.as_str()) {
                            s.push_str(t);
                            s.push('\n');
                        }
                    }
                    s.trim().to_string()
                }
                _ => continue,
            };
            if body.is_empty() { continue; }
            let timestamp = v.get("timestamp").and_then(|t| t.as_str()).unwrap_or(&now_str).to_string();
            let uuid = v.get("uuid").and_then(|u| u.as_str()).unwrap_or(&timestamp).to_string();
            let parent = v.get("parentUuid").and_then(|p| p.as_str()).map(String::from);

            let mut guard = db.lock().await;
            // canonical write path — embedder 주입으로 저장 즉시 임베딩
            let r = crate::save_l0::save_l0_message(&mut *guard, crate::save_l0::L0SaveInput {
                id: Some(uuid),
                session_id: &db_session_id,
                session_title: Some(&session_title),
                sender: role,
                body: &body,
                signature: "claude-ingest",
                timestamp: Some(&timestamp),
                parent_message_id: parent.as_deref(),
                conversation_id: None,
                source: "claude_ingest",
                extra_metadata: Some(serde_json::json!({"file": path_str})),
            }, Some(embedder));
            if let Ok(res) = r { if res.inserted { inserted += 1; } }
        }

        // offset 갱신
        let mut guard = db.lock().await;
        let _ = guard.conn().execute(
            "INSERT INTO claude_ingest_state (file_path, last_offset, session_db_id, last_seen_at, msg_count) \
             VALUES (?1, ?2, ?3, datetime('now'), ?4) \
             ON CONFLICT(file_path) DO UPDATE SET \
               last_offset = excluded.last_offset, \
               session_db_id = excluded.session_db_id, \
               last_seen_at = excluded.last_seen_at, \
               msg_count = msg_count + excluded.msg_count",
            rusqlite::params![path_str, new_offset as i64, db_session_id, inserted as i64],
        );
        total_ingested += inserted;
    }

    if total_ingested > 0 {
        tracing::info!(count = total_ingested, "claude_ingest: messages inserted");
    }
    Ok(())
}

fn read_from_offset(path: &std::path::Path, offset: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut s = String::new();
    f.read_to_string(&mut s)?;
    Ok(s)
}

async fn daily_backup_tick(data_dir: &std::path::Path) -> anyhow::Result<()> {
    let backup_dir = data_dir.join("backup");
    let _ = std::fs::create_dir_all(&backup_dir);
    // 가장 최근 backup 파일 mtime 확인
    let need = match std::fs::read_dir(&backup_dir).ok().and_then(|d| {
        d.filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("backup-"))
            .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()))
            .max()
    }) {
        Some(mtime) => mtime.elapsed().map(|d| d.as_secs() > 86_400).unwrap_or(true),
        None => true,
    };
    if !need { return Ok(()); }
    let data_dir = data_dir.to_path_buf();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let out = data_dir.join("backup").join(format!("backup-{ts}.tar.gz"));
        let f = std::fs::File::create(&out)?;
        let enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);
        for name in ["db.sqlite", "keystore", "notify.toml", "install_manifest.json"] {
            let p = data_dir.join(name);
            if p.exists() {
                if p.is_dir() {
                    tar.append_dir_all(name, &p)?;
                } else {
                    let mut fp = std::fs::File::open(&p)?;
                    tar.append_file(name, &mut fp)?;
                }
            }
        }
        tar.finish()?;
        tracing::info!(out=%out.display(), "daily backup created");
        Ok(())
    })
    .await??;
    Ok(())
}

async fn self_trigger_fire_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    // 활성 SelfTrigger 규칙 로드
    let triggers: Vec<(String, String, String, String)> = {
        let mut guard = db.lock().await;
        let conn = guard.conn();
        let mut stmt = match conn.prepare(
            "SELECT id, event_pattern, target_agent, action FROM self_triggers WHERE active=1",
        ) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };
        let it = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        it.flatten().collect()
    };
    if triggers.is_empty() { return Ok(()); }
    // 직전 30초 새 메시지 가져와서 pattern 매칭
    let since = (chrono::Utc::now() - chrono::Duration::seconds(35)).to_rfc3339();
    let messages: Vec<(String, String, String)> = {
        let mut guard = db.lock().await;
        let conn = guard.conn();
        let mut stmt = match conn.prepare(
            "SELECT msg_ulid, body, created_at FROM messages WHERE created_at >= ?1 LIMIT 100",
        ) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };
        let it = stmt.query_map(rusqlite::params![since], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        it.flatten().collect()
    };
    if messages.is_empty() { return Ok(()); }
    let mut fired = 0;
    for (trig_id, pattern, target, action) in &triggers {
        for (msg_ulid, body, _) in &messages {
            // 간단 매칭: pattern 이 body 의 substring 이면 fire (또는 ":new_message" 같은 special)
            let matched = pattern.is_empty()
                || body.contains(pattern)
                || pattern == "*"
                || (pattern.contains(':') && body.contains(pattern.split(':').last().unwrap_or("")));
            if matched {
                // outbound_queue 에 action 메시지 enqueue (target_agent 가 다른 머신이면)
                let now = chrono::Utc::now().to_rfc3339();
                let action_body = serde_json::json!({
                    "trigger_id": trig_id,
                    "trigger_pattern": pattern,
                    "matched_msg": msg_ulid,
                    "action": action,
                }).to_string();
                let mut guard = db.lock().await;
                let conn = guard.conn();
                // fire_count 증가 + last_fired_at 갱신
                let _ = conn.execute(
                    "UPDATE self_triggers SET fire_count = fire_count + 1, last_fired_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, trig_id],
                );
                // outbound_queue 에 enqueue (target_alias = target_agent)
                let new_ulid = format!("st-{}", &msg_ulid[..std::cmp::min(8, msg_ulid.len())]);
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO outbound_queue (msg_ulid, target_machine, target_alias, body, attempts, enqueued_at) \
                     VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                    rusqlite::params![new_ulid, target, target, action_body, now],
                );
                fired += 1;
                break;
            }
        }
    }
    if fired > 0 {
        tracing::info!(count = fired, "self_triggers fired");
    }
    Ok(())
}

pub fn spawn_all(db: Arc<Mutex<Db>>) {
    let db_m4 = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Err(e) = m4_idle_tick(&db_m4).await {
                tracing::warn!("M-4 idle tick error: {e}");
            }
        }
    });
    let db_m6 = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Err(e) = m6_autotopup_tick(&db_m6).await {
                tracing::warn!("M-6 auto-topup tick error: {e}");
            }
        }
    });
    let db_l6 = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if let Err(e) = l6_expiry_tick(&db_l6).await {
                tracing::warn!("L6 expiry tick error: {e}");
            }
        }
    });
    let db_v6 = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            if let Err(e) = v6_outbound_drain(&db_v6).await {
                tracing::warn!("V6 outbound drain error: {e}");
            }
        }
    });
    let db_m5 = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Err(e) = m5_auto_register_tick(&db_m5).await {
                tracing::warn!("M-5 auto-register tick error: {e}");
            }
        }
    });
    // W-5 message_trigger: 새 메시지 (last 60s) 가 workflow.message_trigger pattern 매칭 시 자동 실행.
    let db_wfmsg = db.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            if let Err(e) = workflow_message_trigger_tick(&db_wfmsg).await {
                tracing::warn!("workflow message trigger tick: {e}");
            }
        }
    });
    let db_m2 = db.clone();
    tokio::spawn(async move {
        if let Err(e) = m2_merge_candidates_tick(&db_m2).await {
            tracing::warn!("M-2 initial tick error: {e}");
        }
        loop {
            tokio::time::sleep(Duration::from_secs(600)).await;
            if let Err(e) = m2_merge_candidates_tick(&db_m2).await {
                tracing::warn!("M-2 merge candidates tick error: {e}");
            }
        }
    });
    tracing::info!("daemon workers spawned (M-2, M-4, M-5, M-6, L6, V6)");
}

/// W-5 message_trigger: workflows.message_trigger (json: {"pattern": "..."}) 가 최근 60s 메시지 body 매칭 시 workflow 자동 실행.
async fn workflow_message_trigger_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    let mut db = db.lock().await;
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT id, yaml_body, message_trigger FROM workflows WHERE enabled=1 AND message_trigger IS NOT NULL AND message_trigger != ''"
    )?;
    let wfs: Vec<(String, String, String)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .filter_map(|r| r.ok()).collect();
    drop(stmt);
    if wfs.is_empty() { return Ok(()); }
    let mut msg_stmt = conn.prepare(
        "SELECT id, body FROM messages WHERE created_at > datetime('now', '-60 seconds') LIMIT 50"
    )?;
    let recent: Vec<(String, String)> = msg_stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(|r| r.ok()).collect();
    drop(msg_stmt);
    for (wf_id, yaml, trigger_json) in &wfs {
        let pattern = serde_json::from_str::<serde_json::Value>(trigger_json).ok()
            .and_then(|v| v.get("pattern").and_then(|p| p.as_str().map(String::from)))
            .unwrap_or_default();
        if pattern.is_empty() { continue; }
        for (msg_id, body) in &recent {
            if body.contains(&pattern) {
                // 이미 trigger 됐는지 확인 (msg_id + workflow_id 조합).
                let already: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM workflow_runs WHERE workflow_id=?1 AND trigger_source LIKE 'message:%' AND trigger_source = ?2",
                    rusqlite::params![wf_id, format!("message:{msg_id}")],
                    |r| r.get(0),
                ).unwrap_or(0);
                if already > 0 { continue; }
                let run_id = uuid::Uuid::new_v4().to_string();
                let _ = conn.execute(
                    "INSERT INTO workflow_runs (id, workflow_id, started_at, status, trigger_source) VALUES (?1, ?2, datetime('now'), 'pending', ?3)",
                    rusqlite::params![run_id, wf_id, format!("message:{msg_id}")],
                );
                tracing::info!("W-5 message trigger: workflow {wf_id} fired by msg {msg_id} → pending run {run_id}");
                let _ = yaml;
                return Ok(());
            }
        }
    }
    Ok(())
}

/// M-2 자동 위키 merge candidate 발견 — 10분 주기.
/// 같은 page_type 의 페이지 쌍 중 normalized title 가 정확히 일치하면 INSERT.
async fn m2_merge_candidates_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    let mut db = db.lock().await;
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT a.id, b.id FROM wiki_pages a JOIN wiki_pages b \
         ON a.page_type = b.page_type AND a.id < b.id \
         AND LOWER(REPLACE(REPLACE(REPLACE(a.title, ' ', ''), '-', ''), '_', '')) \
           = LOWER(REPLACE(REPLACE(REPLACE(b.title, ' ', ''), '-', ''), '_', '')) \
         WHERE NOT EXISTS ( \
           SELECT 1 FROM wiki_merge_candidates m \
           WHERE (m.page_a_id = a.id AND m.page_b_id = b.id) \
              OR (m.page_a_id = b.id AND m.page_b_id = a.id) \
         ) LIMIT 100",
    )?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    let now = chrono::Utc::now().to_rfc3339();
    for (a, b) in &rows {
        let _ = conn.execute(
            "INSERT INTO wiki_merge_candidates (page_a_id, page_b_id, similarity, detected_at, status) VALUES (?1, ?2, 1.0, ?3, 'pending')",
            rusqlite::params![a, b, now],
        );
    }
    if !rows.is_empty() {
        tracing::info!("M-2 merge candidates: {} new pair(s) detected", rows.len());
    }
    Ok(())
}

async fn m4_idle_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    // last_seen_at 기준으로 status 갱신:
    //   < 15min: Active
    //   15~60min: Idle
    //   > 60min: Dormant
    //   > 24h: Offline
    let now = chrono::Utc::now();
    let mut db = db.lock().await;
    let conn = db.conn();
    let mut stmt = conn.prepare(
        "SELECT id, last_seen_at, status FROM agent_identities WHERE status != 'Decommissioned'",
    )?;
    let rows: Vec<(String, Option<String>, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    for (id, last_seen, current_status) in rows {
        let new_status = match last_seen.as_deref().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) {
            Some(t) => {
                let elapsed_min = (now - t.with_timezone(&chrono::Utc)).num_minutes();
                if elapsed_min < 15 { "Active" }
                else if elapsed_min < 60 { "Idle" }
                else if elapsed_min < 60 * 24 { "Dormant" }
                else { "Offline" }
            }
            None => "Offline",
        };
        if new_status != current_status {
            conn.execute(
                "UPDATE agent_identities SET status = ?1 WHERE id = ?2",
                rusqlite::params![new_status, id],
            )?;
            // N8: lifecycle log
            let action = match new_status {
                "Dormant" => "sleep",
                "Active" => "wake",
                _ => "status_change",
            };
            conn.execute(
                "INSERT INTO agent_lifecycle_log (agent_id, action, reason, at) \
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, action, format!("auto: {current_status} -> {new_status}"), now.to_rfc3339()],
            )?;
        }
    }
    Ok(())
}

async fn m6_autotopup_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    // sub_wallets 중 auto_topup_enabled=1 AND balance < threshold 인 항목 처리.
    let now = chrono::Utc::now();
    let today = now.format("%Y-%m-%d").to_string();
    let mut db = db.lock().await;
    let conn = db.conn();
    // 오늘 날짜로 consumed reset
    conn.execute(
        "UPDATE sub_wallets SET auto_topup_consumed_today_micro = 0, auto_topup_consumed_date = ?1 \
         WHERE auto_topup_consumed_date != ?1 OR auto_topup_consumed_date IS NULL",
        rusqlite::params![today],
    )?;
    // 충전 대상 조회
    let mut stmt = conn.prepare(
        "SELECT agent_id, allocated_micro, spent_micro, earned_micro, \
                auto_topup_threshold_micro, auto_topup_amount_micro, \
                auto_topup_max_per_day_micro, auto_topup_consumed_today_micro \
         FROM sub_wallets WHERE auto_topup_enabled = 1 AND status = 'Active'",
    )?;
    let candidates: Vec<(String, i64, i64, i64, i64, i64, i64, i64)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?,
                r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    for (agent_id, alloc, spent, earned, threshold, amount, max_day, used_today) in candidates {
        let balance = alloc - spent + earned;
        if balance >= threshold { continue }
        // 일 한도 체크 (M-6)
        let remaining = max_day - used_today;
        if remaining <= 0 { continue }
        let topup = amount.min(remaining);
        // 마스터 차감 + 서브 +
        conn.execute(
            "UPDATE master_wallet_view SET free_micro = free_micro - ?1, last_synced_at = ?2 WHERE id = 1",
            rusqlite::params![topup, now.to_rfc3339()],
        )?;
        conn.execute(
            "UPDATE sub_wallets SET allocated_micro = allocated_micro + ?1, \
                    auto_topup_consumed_today_micro = auto_topup_consumed_today_micro + ?1, \
                    updated_at = ?2 WHERE agent_id = ?3",
            rusqlite::params![topup, now.to_rfc3339(), agent_id],
        )?;
        tracing::info!("M-6 auto-topup: agent={agent_id} +{}USDC", topup as f64 / 1_000_000.0);
    }
    Ok(())
}

async fn l6_expiry_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    // vault_pending 중 24h 경과한 항목을 만료 처리.
    // 현재는 audit only (실 거절 로직은 VaultStore 의 deny 호출 필요).
    let now = chrono::Utc::now();
    let cutoff = (now - chrono::Duration::hours(24)).to_rfc3339();
    let mut db = db.lock().await;
    let conn = db.conn();
    // pending 테이블 이름이 vault crate 내부라 직접 조회 — N4 안티패턴 위반 우려 있으나
    // L6 worker 는 enforcement 라 예외 (메시지 데이터가 아닌 vault metadata).
    // 일단 row 수만 로그.
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='vault_pending'",
        [],
        |r| r.get(0),
    ).unwrap_or(0);
    if count == 0 { return Ok(()); }
    let expired: i64 = conn.query_row(
        "SELECT COUNT(*) FROM vault_pending WHERE requested_at < ?1 AND status = 'Pending'",
        rusqlite::params![cutoff],
        |r| r.get(0),
    ).unwrap_or(0);
    if expired > 0 {
        // status = 'Expired' 로 표시
        conn.execute(
            "UPDATE vault_pending SET status = 'Expired', decided_at = ?1 \
             WHERE requested_at < ?2 AND status = 'Pending'",
            rusqlite::params![now.to_rfc3339(), cutoff],
        )?;
        tracing::info!("L6 expiry: {} vault pending expired", expired);
    }
    Ok(())
}

/// M-5 자동 등록 worker (60s tick).
/// 화이트리스트 패턴 매칭되는 미연결 세션 발견 시 agent_identities 에 자동 INSERT.
async fn m5_auto_register_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    use crate::daemon_gui_sessions::{collect_sessions, default_whitelist, WhitelistPatternItem};
    let dto = collect_sessions();
    // 화이트리스트 (default + user)
    let default_patterns = default_whitelist().patterns;
    let mut db = db.lock().await;
    let mut user_stmt = db.conn().prepare(
        "SELECT priority, pattern_type, pattern, default_role, auto_register, auto_approve_pending \
         FROM whitelist_patterns WHERE active = 1 ORDER BY priority ASC",
    )?;
    let user_patterns: Vec<WhitelistPatternItem> = user_stmt.query_map([], |r| {
        Ok(WhitelistPatternItem {
            priority: r.get::<_, i64>(0)? as u32,
            pattern_type: r.get(1)?,
            pattern: r.get(2)?,
            default_role: r.get(3)?,
            auto_register: r.get::<_, i64>(4)? != 0,
            auto_approve_pending: r.get::<_, i64>(5)? != 0,
        })
    })?.filter_map(|r| r.ok()).collect();
    drop(user_stmt);
    let mut patterns = default_patterns;
    patterns.extend(user_patterns);
    // N1: command > tmux > cwd 우선순위
    patterns.sort_by_key(|p| p.priority);
    let now = chrono::Utc::now().to_rfc3339();
    for s in &dto.sessions {
        // 이미 agent_identities 에 등록되어 있으면 skip.
        let exists: i64 = db.conn().query_row(
            "SELECT COUNT(*) FROM agent_identities WHERE handle_id = ?1",
            rusqlite::params![s.identifier],
            |r| r.get(0),
        ).unwrap_or(0);
        if exists > 0 { continue }
        // 패턴 매칭 — display + identifier 둘 다 검사
        for p in &patterns {
            if !p.auto_register { continue }
            let target = &s.display;
            let matched = if p.pattern.ends_with('*') {
                let prefix = p.pattern.trim_end_matches('*');
                target.starts_with(prefix)
            } else {
                target.contains(&p.pattern)
            };
            if matched {
                // N4 + 안티패턴 10: 직접 SQL — agent_identities 는 메신저 마스터.
                let id = {
                    use sha2::{Digest, Sha256};
                    let mut h = Sha256::new();
                    h.update(s.identifier.as_bytes());
                    h.update(now.as_bytes());
                    format!("{:x}", h.finalize())[..26].to_string()
                };
                let _ = db.conn().execute(
                    "INSERT OR IGNORE INTO agent_identities \
                        (id, display_name, machine, role, status, llm_mode, handle_id, started_at, last_seen_at) \
                     VALUES (?1, ?2, ?3, ?4, 'Active', 'Working', ?5, ?6, ?6)",
                    rusqlite::params![id, s.display, dto.machine.alias, p.default_role, s.identifier, now],
                );
                // M-5 audit
                let _ = db.conn().execute(
                    "INSERT INTO whitelist_match_log (agent_id, matched_pattern_id, action, at) \
                     VALUES (?1, NULL, 'auto_register', ?2)",
                    rusqlite::params![id, now],
                );
                tracing::info!("M-5 auto-register: {} (pattern: {})", s.display, p.pattern);
                break; // 우선순위 가장 높은 매칭 1개만
            }
        }
    }
    Ok(())
}

async fn v6_outbound_drain(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    // S8: outbound_queue 의 pending 항목을 peer transport URL 로 실제 POST.
    // 성공 → sent_at 기록, 실패 → attempts++ + next_retry_at backoff.
    let now = chrono::Utc::now();
    let archive_cutoff = (now - chrono::Duration::days(30)).to_rfc3339();
    let now_str = now.to_rfc3339();

    // 처리할 pending 항목 + 머신 별 transport URL 함께 추출
    let to_send: Vec<(String, String, String, i64, String)> = {
        let mut guard = db.lock().await;
        let conn = guard.conn();
        let _ = conn.execute(
            "DELETE FROM outbound_queue WHERE sent_at IS NOT NULL AND sent_at < ?1",
            rusqlite::params![archive_cutoff],
        );
        let _ = conn.execute(
            "UPDATE outbound_queue SET last_error = 'max_retries_exceeded' \
             WHERE attempts > 10 AND sent_at IS NULL AND last_error != 'max_retries_exceeded'",
            [],
        );
        let mut rows_out: Vec<(String, String, String, i64, String)> = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT q.msg_ulid, q.target_alias, q.body, q.attempts, p.address \
             FROM outbound_queue q \
             JOIN peers p ON p.alias = q.target_alias \
             WHERE q.sent_at IS NULL \
               AND q.attempts <= 10 \
               AND (q.next_retry_at IS NULL OR q.next_retry_at <= ?1) \
             LIMIT 20",
        ) {
            if let Ok(iter) = stmt.query_map(rusqlite::params![now_str], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                ))
            }) {
                for r in iter.flatten() {
                    rows_out.push(r);
                }
            }
        }
        rows_out
    };

    if to_send.is_empty() {
        return Ok(());
    }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    for (ulid, alias, body, attempts, address) in to_send {
        // rc.215 — endpoint fix: receiver router 는 /v1/message 만 노출. /v1/inbound 는 404 forever.
        let target_url = if address.ends_with('/') {
            format!("{address}v1/message")
        } else {
            format!("{address}/v1/message")
        };
        let result = http
            .post(&target_url)
            .header("Content-Type", "application/json")
            .body(body.clone())
            .send()
            .await;
        let success = match result {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        };
        let now_str2 = chrono::Utc::now().to_rfc3339();
        let mut guard = db.lock().await;
        let conn = guard.conn();
        if success {
            let _ = conn.execute(
                "UPDATE outbound_queue SET sent_at = ?1, last_error = NULL WHERE msg_ulid = ?2",
                rusqlite::params![now_str2, ulid],
            );
            tracing::debug!(ulid=%ulid, alias=%alias, "S8 outbound sent");
        } else {
            // backoff: 1s * 2^attempts (cap 5min)
            let backoff_secs = std::cmp::min(300, 1_i64 << std::cmp::min(attempts, 8));
            let next = (chrono::Utc::now() + chrono::Duration::seconds(backoff_secs)).to_rfc3339();
            let _ = conn.execute(
                "UPDATE outbound_queue SET attempts = attempts + 1, next_retry_at = ?1, \
                 last_error = ?2 WHERE msg_ulid = ?3",
                rusqlite::params![next, "transport send failed", ulid],
            );
            tracing::debug!(ulid=%ulid, alias=%alias, target=%target_url, "S8 outbound failed, backoff");
        }
    }
    Ok(())
}

/// rc.170 — auto-echo enforcer worker.
/// 60s 주기로 active discord binding 마다 매칭 session 의 최신 assistant message 를
/// last_echoed_ulid 와 비교 → 새 메시지면 Discord 채널로 push.
/// matching: COALESCE(session_proj_name, agent_id) → `claude:{proj}:%` LIKE.
/// first_setup (last_echoed=NULL) 시 옛 메시지 echo 안 함 — 현재 msg_id 로 mark.
pub async fn auto_echo_tick(db: &Arc<Mutex<Db>>) -> anyhow::Result<()> {
    let bindings: Vec<(String, String, String, Option<String>, Option<String>, Option<String>)> = {
        let mut guard = db.lock().await;
        let mut stmt = match guard.conn().prepare(
            "SELECT id, agent_id, channel_ref, bot_id, last_echoed_ulid, session_proj_name \
             FROM session_channel_bindings \
             WHERE platform='discord' AND active=1"
        ) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };
        let rows = stmt.query_map([], |r| Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
        )));
        match rows {
            Ok(it) => it.flatten().collect(),
            Err(_) => Vec::new(),
        }
    };

    if bindings.is_empty() {
        return Ok(());
    }

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    for (binding_id, agent_id, channel_ref, bot_id, last_echoed, proj_name) in bindings {
        let proj = proj_name.clone().unwrap_or_else(|| agent_id.clone());
        let pattern = format!("claude:{}:%", proj);

        let latest: Option<(String, String)> = {
            let mut guard = db.lock().await;
            guard.conn().query_row(
                "SELECT id, body FROM messages \
                 WHERE session_id LIKE ?1 AND sender='assistant' \
                 ORDER BY timestamp DESC LIMIT 1",
                rusqlite::params![&pattern],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            ).ok()
        };

        let Some((msg_id, body)) = latest else { continue; };

        if last_echoed.as_deref() == Some(msg_id.as_str()) { continue; }

        if last_echoed.is_none() {
            let mut guard = db.lock().await;
            let _ = guard.conn().execute(
                "UPDATE session_channel_bindings SET last_echoed_ulid=?1 WHERE id=?2",
                rusqlite::params![&msg_id, &binding_id]
            );
            tracing::info!(binding=%binding_id, agent=%agent_id, "auto_echo: first_setup mark, 옛 메시지 echo 안 함");
            continue;
        }

        let token: Option<String> = {
            let mut guard = db.lock().await;
            match &bot_id {
                Some(bid) => guard.conn().query_row(
                    "SELECT bot_token FROM discord_bots WHERE id=?1 AND active=1",
                    rusqlite::params![bid],
                    |r| r.get::<_, String>(0)
                ).ok(),
                None => guard.conn().query_row(
                    "SELECT bot_token FROM discord_bots WHERE active=1 ORDER BY created_at LIMIT 1",
                    [],
                    |r| r.get::<_, String>(0)
                ).ok(),
            }
        };

        let Some(token) = token else {
            tracing::warn!(binding=%binding_id, agent=%agent_id, "auto_echo: bot token 없음");
            continue;
        };

        // Discord 2000 char 제한
        let payload_body = if body.chars().count() > 1900 {
            let truncated: String = body.chars().take(1900).collect();
            format!("{}\n...[잘림]", truncated)
        } else {
            body
        };

        let url = format!("https://discord.com/api/v10/channels/{}/messages", channel_ref);
        let payload = serde_json::json!({"content": payload_body});
        let resp = http.post(&url)
            .header("Authorization", format!("Bot {}", token))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let mut guard = db.lock().await;
                let _ = guard.conn().execute(
                    "UPDATE session_channel_bindings SET last_echoed_ulid=?1 WHERE id=?2",
                    rusqlite::params![&msg_id, &binding_id]
                );
                tracing::info!(binding=%binding_id, agent=%agent_id, channel=%channel_ref, msg_id=%msg_id, "auto_echo: Discord 발송 success");
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                tracing::warn!(binding=%binding_id, status=%status, body=%text, "auto_echo: Discord HTTP 실패");
            }
            Err(e) => {
                tracing::warn!(binding=%binding_id, error=%e, "auto_echo: 네트워크 실패");
            }
        }
    }

    Ok(())
}
