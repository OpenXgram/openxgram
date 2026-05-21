//! UI-MESSENGER-SPEC v1.4 §20 — 오케스트레이션 워크플로 engine.
//!
//! YAML → struct → DAG → step 실행 (action 호출) → workflow_runs / workflow_step_logs 갱신.
//!
//! W-1 YAML parse · W-2 depends_on DAG · W-3 human_approval_at · W-6 {{steps.X.output}} interp
//! W-7 on_error · W-8 cost_limit_usdc · W-10 orchestrator
//!
//! cron / 메시지 트리거 (W-4/W-5/W-9) = daemon scheduler 가 별도 spawn (`schedule_workflow_cron`).

use openxgram_db::Db;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowYaml {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub steps: Vec<StepDef>,
    #[serde(default)]
    pub on_error: Vec<HashMap<String, serde_yaml::Value>>,
    #[serde(default)]
    pub cost_limit_usdc: Option<f64>,
    #[serde(default)]
    pub human_approval_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StepDef {
    pub id: String,
    #[serde(default)]
    pub agent: String,
    pub action: String,
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

pub struct EngineResult {
    pub status: String,           // success | failed | aborted | waiting_human
    pub error: Option<String>,
    pub total_cost: f64,
    pub step_outputs: HashMap<String, String>,
}

/// YAML 검증 + 파싱.
pub fn parse_yaml(yaml: &str) -> Result<WorkflowYaml, String> {
    serde_yaml::from_str(yaml).map_err(|e| format!("YAML parse: {e}"))
}

/// {{steps.X.output}} interpolation (W-6).
fn interpolate(template: &str, outputs: &HashMap<String, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in outputs {
        out = out.replace(&format!("{{{{steps.{k}.output}}}}"), v);
    }
    out
}

/// Topological sort (W-2 depends_on DAG).
fn topo_sort(steps: &[StepDef]) -> Result<Vec<String>, String> {
    let mut visited = std::collections::HashSet::new();
    let mut order = Vec::new();
    let mut visiting = std::collections::HashSet::new();
    let by_id: HashMap<&str, &StepDef> = steps.iter().map(|s| (s.id.as_str(), s)).collect();

    fn visit<'a>(
        id: &'a str,
        by_id: &HashMap<&'a str, &'a StepDef>,
        visited: &mut std::collections::HashSet<String>,
        visiting: &mut std::collections::HashSet<String>,
        order: &mut Vec<String>,
    ) -> Result<(), String> {
        if visited.contains(id) {
            return Ok(());
        }
        if visiting.contains(id) {
            return Err(format!("cycle detected at {id}"));
        }
        visiting.insert(id.to_string());
        let step = by_id.get(id).ok_or_else(|| format!("step {id} not found"))?;
        for dep in &step.depends_on {
            visit(dep, by_id, visited, visiting, order)?;
        }
        visiting.remove(id);
        visited.insert(id.to_string());
        order.push(id.to_string());
        Ok(())
    }

    for s in steps {
        visit(&s.id, &by_id, &mut visited, &mut visiting, &mut order)?;
    }
    Ok(order)
}

/// 단계 action 실행. llm_call = Ollama 진짜 호출. 나머지 mock.
async fn execute_step(step: &StepDef, input: &str) -> Result<(String, f64), String> {
    let mut cost = 0.001;
    let output = match step.action.as_str() {
        "echo" => format!("[echo:{}] {}", step.agent, input),
        "web_search" => {
            // DuckDuckGo lite HTML — 외부 API key 불필요.
            let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30)).build()
                .map_err(|e| format!("client: {e}"))?;
            let url = format!("https://html.duckduckgo.com/html/?q={}", urlencoding::encode(input));
            let resp = client.get(&url).header("User-Agent", "Mozilla/5.0 openxgram-workflow").send().await
                .map_err(|e| format!("ddg: {e}"))?;
            let body = resp.text().await.unwrap_or_default();
            // 단순 추출: <a class="result__a" href="...">텍스트</a> 의 텍스트 + URL 첫 5개.
            let re = regex::Regex::new(r#"<a[^>]+class="result__a"[^>]+href="([^"]+)"[^>]*>([^<]+)</a>"#).unwrap();
            let hits: Vec<String> = re.captures_iter(&body).take(5)
                .map(|c| format!("{} — {}", &c[2].trim(), &c[1])).collect();
            cost = 0.002;
            if hits.is_empty() { format!("[web_search: 0 results]") } else { hits.join("\n") }
        }
        "llm_call" => {
            let base = std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| "http://localhost:11434".into());
            let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gemma3:4b".into());
            let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build()
                .map_err(|e| format!("client: {e}"))?;
            let resp = client.post(format!("{base}/api/generate"))
                .json(&serde_json::json!({"model": model, "prompt": input, "stream": false}))
                .send().await.map_err(|e| format!("ollama: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("ollama HTTP {}", resp.status()));
            }
            let j: serde_json::Value = resp.json().await.map_err(|e| format!("ollama json: {e}"))?;
            cost = 0.005;
            j.get("response").and_then(|r| r.as_str()).unwrap_or("(no response)").to_string()
        }
        "email" => {
            // SMTP via env SMTP_HOST / SMTP_PORT / SMTP_USER / SMTP_PASS / SMTP_FROM. 미설정 시 mock.
            let host = std::env::var("SMTP_HOST").ok();
            let to = step.to.as_deref().unwrap_or("");
            let body = step.body.as_deref().unwrap_or(input);
            if to.is_empty() {
                return Err("email: 'to' 필수".into());
            }
            if host.is_none() {
                format!("[email mock — SMTP_HOST 미설정] to={to} body='{}'", body.chars().take(80).collect::<String>())
            } else {
                let host = host.unwrap();
                let port: u16 = std::env::var("SMTP_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(587);
                let user = std::env::var("SMTP_USER").unwrap_or_default();
                let pass = std::env::var("SMTP_PASS").unwrap_or_default();
                let from = std::env::var("SMTP_FROM").unwrap_or_else(|_| user.clone());
                use lettre::{Message, SmtpTransport, Transport};
                use lettre::transport::smtp::authentication::Credentials;
                let from_c = from.clone(); let to_c = to.to_string(); let body_c = body.to_string();
                let subject = format!("workflow {} - {}", step.agent, step.id);
                let host_c = host.clone(); let user_c = user.clone(); let pass_c = pass.clone();
                tokio::task::spawn_blocking(move || -> Result<(), String> {
                    let email = Message::builder()
                        .from(from_c.parse().map_err(|e: lettre::address::AddressError| format!("from: {e}"))?)
                        .to(to_c.parse().map_err(|e: lettre::address::AddressError| format!("to: {e}"))?)
                        .subject(subject)
                        .body(body_c)
                        .map_err(|e| format!("build: {e}"))?;
                    let creds = Credentials::new(user_c, pass_c);
                    let mailer = SmtpTransport::relay(&host_c).map_err(|e| format!("relay: {e}"))?
                        .port(port).credentials(creds).build();
                    mailer.send(&email).map_err(|e| format!("send: {e}"))?;
                    Ok(())
                }).await.map_err(|e| format!("join: {e}"))??;
                cost = 0.001;
                format!("[email sent] to={to} subject='workflow {} - {}'", step.agent, step.id)
            }
        }
        other => format!("[unsupported:{}] {}={}", other, step.agent, input),
    };
    Ok((output, cost))
}

/// 실 엔진 실행. workflow_runs + workflow_step_logs 갱신.
pub async fn run_workflow(
    db: &mut Db,
    workflow_id: &str,
    run_id: &str,
    yaml_body: &str,
) -> EngineResult {
    let mut total_cost = 0.0;
    let mut step_outputs: HashMap<String, String> = HashMap::new();

    let wf = match parse_yaml(yaml_body) {
        Ok(w) => w,
        Err(e) => {
            let _ = db.conn().execute(
                "UPDATE workflow_runs SET status='failed', error=?1, finished_at=datetime('now') WHERE id=?2",
                rusqlite::params![format!("YAML parse: {e}"), run_id],
            );
            return EngineResult {
                status: "failed".into(), error: Some(e), total_cost: 0.0, step_outputs,
            };
        }
    };

    let order = match topo_sort(&wf.steps) {
        Ok(o) => o,
        Err(e) => {
            let _ = db.conn().execute(
                "UPDATE workflow_runs SET status='failed', error=?1, finished_at=datetime('now') WHERE id=?2",
                rusqlite::params![format!("DAG: {e}"), run_id],
            );
            return EngineResult {
                status: "failed".into(), error: Some(e), total_cost: 0.0, step_outputs,
            };
        }
    };

    let by_id: HashMap<String, StepDef> = wf.steps.iter().map(|s| (s.id.clone(), s.clone())).collect();

    for step_id in &order {
        let step = match by_id.get(step_id) {
            Some(s) => s,
            None => continue,
        };
        // W-3 human approval gate
        if let Some(ha) = &wf.human_approval_at {
            if ha == step_id {
                let _ = db.conn().execute(
                    "UPDATE workflow_runs SET status='waiting_human', current_step=?1 WHERE id=?2",
                    rusqlite::params![step_id, run_id],
                );
                return EngineResult {
                    status: "waiting_human".into(), error: None, total_cost, step_outputs,
                };
            }
        }
        // W-6 input interpolation
        let input = interpolate(&step.input, &step_outputs);
        let _ = db.conn().execute(
            "INSERT INTO workflow_step_logs (run_id, step_name, started_at, status) VALUES (?1, ?2, datetime('now'), 'running')",
            rusqlite::params![run_id, step_id],
        );
        let _ = db.conn().execute(
            "UPDATE workflow_runs SET current_step=?1 WHERE id=?2",
            rusqlite::params![step_id, run_id],
        );

        match execute_step(step, &input).await {
            Ok((output, cost)) => {
                total_cost += cost;
                // W-8 cost limit
                if let Some(limit) = wf.cost_limit_usdc {
                    if total_cost > limit {
                        let _ = db.conn().execute(
                            "UPDATE workflow_runs SET status='aborted', error=?1, total_cost=?2, finished_at=datetime('now') WHERE id=?3",
                            rusqlite::params![format!("W-8 cost limit exceeded: {total_cost} > {limit}"), total_cost, run_id],
                        );
                        return EngineResult {
                            status: "aborted".into(), error: Some(format!("cost limit {limit} exceeded")), total_cost, step_outputs,
                        };
                    }
                }
                let _ = db.conn().execute(
                    "UPDATE workflow_step_logs SET status='success', output_preview=?1, cost=?2, finished_at=datetime('now') WHERE run_id=?3 AND step_name=?4 AND status='running'",
                    rusqlite::params![output.chars().take(200).collect::<String>(), cost, run_id, step_id],
                );
                step_outputs.insert(step_id.clone(), output);
            }
            Err(e) => {
                let _ = db.conn().execute(
                    "UPDATE workflow_step_logs SET status='failed', output_preview=?1, finished_at=datetime('now') WHERE run_id=?2 AND step_name=?3 AND status='running'",
                    rusqlite::params![format!("ERROR: {e}"), run_id, step_id],
                );
                // W-7 on_error abort
                let _ = db.conn().execute(
                    "UPDATE workflow_runs SET status='failed', error=?1, total_cost=?2, finished_at=datetime('now') WHERE id=?3",
                    rusqlite::params![format!("step {step_id}: {e}"), total_cost, run_id],
                );
                return EngineResult {
                    status: "failed".into(), error: Some(e), total_cost, step_outputs,
                };
            }
        }
    }

    let _ = db.conn().execute(
        "UPDATE workflow_runs SET status='success', total_cost=?1, finished_at=datetime('now') WHERE id=?2",
        rusqlite::params![total_cost, run_id],
    );
    let _ = workflow_id; // workflow_id 향후 자식 process 시 사용 가능
    EngineResult {
        status: "success".into(), error: None, total_cost, step_outputs,
    }
}
