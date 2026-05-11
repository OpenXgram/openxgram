# OpenXgram installer — Windows PowerShell.
#
# Usage (PowerShell):
#   irm https://openxgram.org/install.ps1 | iex
#   $env:OPENXGRAM_VERSION="v0.2.0-rc.15"; irm https://openxgram.org/install.ps1 | iex
#
# Privacy: GitHub Releases asset 만 download + SHA256 검증 후 install.
#   텔레메트리 / 통계 / 외부 보고 0.

$ErrorActionPreference = 'Stop'

$REPO     = 'OpenXgram/openxgram'
$VERSION  = if ($env:OPENXGRAM_VERSION) { $env:OPENXGRAM_VERSION } else { 'latest' }
$INSTALL  = if ($env:OPENXGRAM_INSTALL_DIR) { $env:OPENXGRAM_INSTALL_DIR } else { Join-Path $env:USERPROFILE 'xgram' }

Write-Host ''
Write-Host '==> OpenXgram installer (Windows)' -ForegroundColor Cyan
Write-Host "    repo    : $REPO"
Write-Host "    version : $VERSION"
Write-Host "    install : $INSTALL"
Write-Host ''

# 1. Resolve version → tag
if ($VERSION -eq 'latest') {
    $api = "https://api.github.com/repos/$REPO/releases/latest"
    Write-Host "==> Step 1: latest tag 조회 — $api"
    try {
        $rel = Invoke-RestMethod -UseBasicParsing -Uri $api
        $tag = $rel.tag_name
    } catch {
        # latest API 가 prerelease 거를 수 있어서 — list 에서 첫 번째 시도
        $rels = Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$REPO/releases"
        $tag = $rels[0].tag_name
    }
} else {
    $tag = $VERSION
}
Write-Host "    → tag = $tag"

# 2. Download URL 구성
$asset   = "xgram-$tag-x86_64-windows.zip"
$dlUrl   = "https://github.com/$REPO/releases/download/$tag/$asset"
$shaUrl  = "$dlUrl.sha256"
$tmpZip  = Join-Path $env:TEMP $asset
$tmpSha  = "$tmpZip.sha256"

Write-Host "==> Step 2: download — $dlUrl"
Invoke-WebRequest -UseBasicParsing -Uri $dlUrl -OutFile $tmpZip
Invoke-WebRequest -UseBasicParsing -Uri $shaUrl -OutFile $tmpSha

# 3. SHA256 검증
Write-Host '==> Step 3: SHA256 검증'
$expected = (Get-Content $tmpSha).Split(' ')[0].ToLower()
$actual   = (Get-FileHash $tmpZip -Algorithm SHA256).Hash.ToLower()
if ($expected -ne $actual) {
    Write-Error "SHA256 불일치 — expected $expected / actual $actual"
    exit 1
}
Write-Host "    ✓ SHA256 일치 ($actual.Substring(0, 12)...)"

# 4. install dir 준비 + 압축 해제
Write-Host "==> Step 4: install → $INSTALL"
if (-not (Test-Path $INSTALL)) {
    New-Item -ItemType Directory -Force -Path $INSTALL | Out-Null
}

# 4a. 잠긴 .exe 가 있으면 Expand-Archive 가 silent skip 함 → 실행 중 프로세스 먼저 종료.
$running = Get-Process -Name xgram, xgram-desktop -ErrorAction SilentlyContinue
if ($running) {
    Write-Host "    → 실행 중인 OpenXgram 프로세스 종료 후 갱신 (재부팅 불필요)"
    foreach ($p in $running) {
        Write-Host "      - $($p.Name) (PID $($p.Id))"
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
    # Windows 가 핸들을 실제로 놓을 때까지 잠깐 대기.
    Start-Sleep -Milliseconds 800
}

# 4b. install dir 통째 삭제 후 재생성 — PS 5.1 의 모든 silent skip 케이스 회피.
#     사이드 케이스 (hidden 속성, ACL, 파일별 잠금 등) 다 우회.
Write-Host "    → install dir 통째 정리: $INSTALL"
if (Test-Path $INSTALL) {
    try {
        Remove-Item -Path $INSTALL -Recurse -Force -ErrorAction Stop
    } catch {
        Write-Error "install dir 삭제 실패 (잠금/권한): $($_.Exception.Message)"
        Write-Error "다음 명령으로 수동 종료 후 재시도: Get-Process xgram, xgram-desktop | Stop-Process -Force"
        exit 1
    }
}
New-Item -ItemType Directory -Force -Path $INSTALL | Out-Null

# 4c. 새 빈 dir 에 압축 해제 — Expand-Archive 만으로도 빈 dir 이라 문제 없음.
Expand-Archive -Path $tmpZip -DestinationPath $INSTALL -Force

# 4c-1. 압축 해제 결과 명시 로그 (디버깅용 — silent skip 즉시 발견).
Write-Host "    → install dir 내용 (압축 해제 직후):"
Get-ChildItem $INSTALL -File | ForEach-Object {
    Write-Host "      - $($_.Name)  $([int]($_.Length/1024))KB  $($_.LastWriteTime)"
}

# 4d. 갱신 검증 — xgram.exe 존재만 확인. LastWriteTime 비교는 zip 내부 시각 vs Get-Date 로컬 시각
#     불일치(timezone) 로 false alarm 발생. step 4b 에서 dir 통째 비웠으니 silent-skip 자체가 불가능.
$xgramExe = Join-Path $INSTALL 'xgram.exe'
if (-not (Test-Path $xgramExe)) {
    Write-Error "xgram.exe 가 install dir 에 없음 — 압축 해제 실패. zip 파일 손상 의심."
    exit 1
}

Remove-Item $tmpZip, $tmpSha -ErrorAction SilentlyContinue

# 5. PATH 영구 추가 (User scope, 이미 있으면 skip)
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$INSTALL*") {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$INSTALL", 'User')
    Write-Host "    ✓ PATH 에 영구 추가 ($INSTALL) — 새 PowerShell 창에서 자동 적용"
} else {
    Write-Host "    (PATH 에 이미 있음)"
}
$env:Path += ";$INSTALL"

# 6. 검증
Write-Host ''
Write-Host '==> 설치 완료' -ForegroundColor Green
& "$INSTALL\xgram.exe" --version

Write-Host ''
Write-Host '다음 단계:' -ForegroundColor Cyan
Write-Host ''
Write-Host '[1] 신원 초기화 (한 번만):'
Write-Host '    xgram init --alias my-laptop'
Write-Host ''
Write-Host '[2] (선택) Discord / Telegram 연결 — 인터랙티브 마법사:'
Write-Host '    xgram setup discord            # webhook + bot token + channel id'
Write-Host '    xgram setup telegram           # bot token + chat id'
Write-Host ''
Write-Host '[3] (선택) Claude Code 등 LLM 에 OpenXgram MCP 서버 등록:'
Write-Host '    xgram mcp-install --scope user        # ~/.claude.json 에 자동 등록'
Write-Host '    xgram identity-inject                  # 프로젝트 CLAUDE.md 에 OpenXgram context 주입'
Write-Host '    # → LLM 이 자연어로 메시지 송수신 가능 (openxgram.* MCP 도구)'
Write-Host ''
Write-Host '[4] daemon + GUI:'
Write-Host '    xgram daemon                   # foreground 또는 백그라운드'
Write-Host '    xgram gui                      # Tauri 데스크탑 창'
Write-Host ''
Write-Host '한 번에 모든 셋업 (인터랙티브 wizard):'
Write-Host '    irm https://openxgram.org/quickstart.ps1 | iex'
Write-Host ''
Write-Host '데모 plan: https://openxgram.org/demo/'
Write-Host ''
