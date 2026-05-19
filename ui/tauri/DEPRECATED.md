# ui/tauri — DEPRECATED (v0.2.0-rc.24~)

이 디렉터리(`ui/tauri/`)의 Tauri 데스크톱 앱(`xgram-desktop`) 은 **폐기됨**.
신규 작업·버그 수정 모두 진행하지 않음. 다음 release 부터 빌드·배포 매트릭스에서도 제거.

## 폐기 이유

| 항목 | Tauri 네이티브 | 웹 GUI (Tailscale Funnel) |
|---|---|---|
| release 빌드 시간 | ~30분 (5 OS cross-compile) | ~1분 (`vite build`) |
| 사용자 다운로드 | ~30MB 추가 binary | 0 (브라우저만) |
| Cloudflare 의존 | 없음 | 없음 |
| HTTPS·도메인 | 별도 setup 필요 | 자동 (`<machine>.tailXXXX.ts.net`) |
| install 흐름 | OS별 별도 launcher | URL 한 줄 |
| 메모리 사용 | WebView 프로세스 상주 | 브라우저 탭 한 개 |

## 대체

- 웹 GUI: `ui/web/` (Solid.js + invoke→fetch)
- 호스팅: Tailscale Funnel (사용자 자신의 tailnet)
- 진입점: `xgram gui` 명령 — `tailscale status --json` 으로 Funnel URL 추출 → 브라우저 자동 실행
- 백엔드: 기존 `daemon` 의 `/v1/gui/*` HTTP API (포트 47302) 그대로 재활용

## 마이그레이션 (사용자)

```bash
# 1) Tailscale 로그인 (한 번만)
sudo tailscale up

# 2) Funnel 활성화 — nginx 가 ui/web 빌드 산출물을 47310 에 서빙한다고 가정
sudo tailscale funnel --bg --https=443 http://localhost:47310

# 3) GUI 실행 — 브라우저 자동 열림
xgram gui
```

기존 `xgram-desktop.exe` / `xgram-desktop` 바이너리를 가진 사용자는 그대로 사용 가능하지만, 다음 release 부터 install tarball 에 동봉되지 않음.

## 워크스페이스 위계

- workspace `Cargo.toml` 의 `exclude = ["ui/tauri"]` 유지 (워크스페이스 빌드에서 제외)
- GitHub Actions `release-binaries.yml` 의 Tauri 빌드 step 제거됨 (rc.24)
- `install.ps1` / `install.sh` 도 `xgram-desktop` 다운로드 부분 제거됨

## 디렉터리 보존 정책

코드는 삭제하지 않음. 다음 이유로 보존:
- 과거 release 사용자가 코드를 참조할 수 있도록
- 미래에 Tauri 가 우월한 솔루션으로 부활할 가능성
- git history 압축으로 인한 정보 손실 방지

신규 PR 가 이 디렉터리를 수정하면 자동 reject. 별도 archive 폴더 이동은 git history 깨짐 우려로 보류.

## 관련 문서

- 정본 PRD: `docs/PRD-OpenXgram.md` v1.2 §4.8 (Beta 웹 GUI / Tailscale Funnel)
- CHANGELOG: `CHANGELOG.md` `[0.2.0-rc.24]` 항목
- 웹 GUI 코드: `ui/web/`
