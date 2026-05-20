//! cargo build prep — `ui/web/dist/` 가 없을 때 placeholder 생성.
//!
//! 배경: `ui_assets.rs` 의 `include_dir!("$CARGO_MANIFEST_DIR/../../ui/web/dist")`
//! 매크로는 컴파일 시 경로가 존재해야 함. CI runner 또는 신규 클론한 개발자가
//! npm build 안 한 상태에서 cargo check 만 돌리면 panic.
//!
//! 이 스크립트는 안전망 — 빈 placeholder index.html 생성. 진짜 빌드 흐름
//! (CI release / 사용자 로컬) 에서는 `cd ui/web && npm run build` 가 먼저
//! 실행돼서 placeholder 가 진짜 dist 로 대체된 후 cargo build 가 매크로 평가.

use std::fs;
use std::path::Path;

fn main() {
    let dist = Path::new("../../ui/web/dist");
    fs::create_dir_all(dist).ok();
    fs::create_dir_all(dist.join("assets")).ok();
    let idx = dist.join("index.html");
    if !idx.exists() {
        let _ = fs::write(
            &idx,
            "<!doctype html>\n<meta charset=utf-8>\n<title>OpenXgram (assets missing)</title>\n\
             <p>Web GUI 자산이 빌드되지 않았습니다.</p>\n\
             <p>Developer: <code>cd ui/web &amp;&amp; npm install &amp;&amp; npm run build</code> 후 <code>cargo build</code></p>\n\
             <p>User: 최신 release 받기 — <code>curl -sSfL https://openxgram.org/install.sh | sh</code></p>\n",
        );
    }
    println!("cargo:rerun-if-changed=../../ui/web/dist");
    println!("cargo:rerun-if-changed=build.rs");
}
