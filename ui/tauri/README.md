# OpenXgram Desktop (Tauri 2.x)

Phase 2 baseline — `xgram doctor` 결과를 시각화하는 desktop 앱.

## 빌드 의존

**Linux (Ubuntu/Debian)**:
```bash
sudo apt install \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libsoup-3.0-dev \
  libssl-dev \
  pkg-config
```

**macOS**: 추가 의존 없음 (Apple WebKit).

**Windows**: WebView2 (Edge 기반) 보통 OS 기본 포함.

## 개발 실행

```bash
cd ui/tauri
cargo install tauri-cli --version "^2"   # 최초 1회
cargo tauri dev                           # 개발 윈도우 (hot reload)
```

## 프로덕션 빌드

```bash
cd ui/tauri
cargo tauri build
# 산출물:
#   - Linux: target/release/bundle/appimage/*.AppImage
#   - macOS: target/release/bundle/dmg/*.dmg
#   - Windows: target/release/bundle/msi/*.msi
```

## 현재 기능

- Status (Doctor) — `xgram doctor --json` 호출 결과 색상 표시
- Version — `xgram version --json` 결과 헤더 표시

## 의존

- 사용자 환경의 `xgram` 바이너리가 PATH 에 있어야 함.
- frontend 는 정적 HTML/JS — Vite/Webpack 등 번들러 미사용 (baseline 단순성).

## Phase 2 후속

- Sessions 탭 (xgram session list/recall)
- Vault 탭 (vault list + pending approve UI)
- 다국어 (ko/en)
- 자동 업데이트 (tauri updater)
- React/Solid 프론트엔드로 마이그레이션 (필요 시)

## 주의

- 워크스페이스 외부 분리 — `cargo test --workspace` 등은 이 crate 를 빌드하지 않음.
- CI 에서 데스크톱 빌드를 검증하려면 별도 job + apt 설치 필요.
