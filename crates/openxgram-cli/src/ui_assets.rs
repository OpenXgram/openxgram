//! Web GUI 정적 자산 임베드 (PRD-OpenXgram v1.3 §4.8).
//!
//! cargo build 시 `ui/web/dist/` 통째로 xgram 바이너리에 포함 → daemon 이
//! `/gui/*` 정적 서빙. nginx 외부 호스팅 불필요. 외부 노출은 Tailscale Funnel
//! (또는 reverse proxy) 위임. dist 가 비어있어도 컴파일 통과 (404 응답).

use axum::{
    body::Body,
    extract::Path,
    http::{header, StatusCode},
    response::Response,
};
use include_dir::{include_dir, Dir};

static UI_DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../../ui/web/dist");

/// 디스크 우선 서빙 디렉토리 (env `XGRAM_GUI_DIR`).
/// 설정돼 있고 디렉토리가 존재하면 요청 시 디스크에서 읽어 서빙 → 프론트 변경 시
/// cargo 재빌드 없이 `dist` 복사만으로 즉시 반영. 없으면 임베드 UI_DIST fallback.
fn disk_dir() -> Option<std::path::PathBuf> {
    let d = std::env::var("XGRAM_GUI_DIR").ok()?;
    let p = std::path::PathBuf::from(d);
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

fn cache_for(path: &str) -> &'static str {
    if path == "index.html" {
        "no-store, no-cache, must-revalidate"
    } else {
        "public, max-age=3600"
    }
}

/// `XGRAM_GUI_DIR` 에서 파일을 읽어 서빙 (path traversal 차단). 없으면 None → 임베드 fallback.
fn try_disk(path: &str) -> Option<Response> {
    let dir = disk_dir()?;
    let safe = path.trim_start_matches('/');
    if safe.contains("..") {
        return None;
    }
    if let Ok(bytes) = std::fs::read(dir.join(safe)) {
        let mime = mime_guess::from_path(safe).first_or_octet_stream();
        return Some(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, cache_for(safe))
                .body(Body::from(bytes))
                .expect("response build"),
        );
    }
    // SPA fallback — 디스크 index.html.
    if let Ok(bytes) = std::fs::read(dir.join("index.html")) {
        return Some(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store, no-cache, must-revalidate")
                .body(Body::from(bytes))
                .expect("response build"),
        );
    }
    None
}

/// `GET /gui` → index.html.
pub async fn gui_root() -> Response {
    serve("index.html")
}

/// `GET /gui/{*path}` — 임베드된 자산 또는 SPA fallback (index.html).
pub async fn gui_asset_path(Path(path): Path<String>) -> Response {
    let p = path.trim_start_matches('/');
    let real = if p.is_empty() { "index.html" } else { p };
    serve(real)
}

fn serve(path: &str) -> Response {
    // 1. 디스크 우선 (XGRAM_GUI_DIR) — 프론트 변경 시 cargo 재빌드 불필요.
    if let Some(resp) = try_disk(path) {
        return resp;
    }
    // 2. 임베드 fallback (기존 동작).
    if let Some(file) = UI_DIST.get_file(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        let cache = if path == "index.html" {
            "no-store, no-cache, must-revalidate"
        } else {
            // hash-named asset → 1h 안전
            "public, max-age=3600"
        };
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .header(header::CACHE_CONTROL, cache)
            .body(Body::from(file.contents()))
            .expect("response build");
    }
    // SPA fallback — hash 자산 외 모든 경로는 index.html (Solid Router 대응).
    if let Some(idx) = UI_DIST.get_file("index.html") {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store, no-cache, must-revalidate")
            .body(Body::from(idx.contents()))
            .expect("response build");
    }
    // dist 비어있음 (개발 환경 또는 빌드 누락).
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(
            "OpenXgram Web GUI assets not embedded.\n\n\
             Developer: cd ui/web && npm install && npm run build, then `cargo build`.\n\
             User: install.sh 다시 실행 또는 최신 release 받기.\n",
        ))
        .expect("response build")
}
