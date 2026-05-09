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
Expand-Archive -Path $tmpZip -DestinationPath $INSTALL -Force
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
Write-Host '다음 단계:'
Write-Host '  xgram init --alias my-laptop      # alias + 패스워드 설정'
Write-Host '  xgram gui                          # Tauri 데스크탑 창'
Write-Host '  xgram daemon                       # foreground 실행 (또는 systemd-task 로 백그라운드)'
Write-Host ''
Write-Host '데모 plan: https://openxgram.org/demo/'
Write-Host ''
