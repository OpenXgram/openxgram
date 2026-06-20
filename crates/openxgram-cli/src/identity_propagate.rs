use anyhow::Result;
use std::path::Path;

/// 데몬→에이전트 신원 갱신 전파. peer_send 파이프라인에 envelope_type="identity_update" 로 실어 보낸다.
/// body 는 JSON: { "alias": ..., "display_name": ..., "role": ... } (변경된 필드만 Some).
pub async fn send_identity_update(
    data_dir: &Path,
    alias: &str,
    display_name: Option<&str>,
    role: Option<&str>,
    password: &str,
) -> Result<()> {
    let payload = serde_json::json!({
        "alias": alias,
        "display_name": display_name,
        "role": role,
    });
    let body = serde_json::to_string(&payload)?;
    crate::peer_send::run_peer_send_with_conv(
        data_dir,
        alias,
        None,
        &body,
        password,
        None,
        Some("identity_update".to_string()),
    )
    .await
}
