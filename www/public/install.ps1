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

# 4b. install dir 의 기존 파일들 모두 명시 삭제 (잠금이면 raise).
#     PS 5.1 Expand-Archive -Force 는 기존 .exe 를 덮어쓰지 못하는 버그가 있어 사전 정리 필요.
$existingFiles = Get-ChildItem -Path $INSTALL -File -Force -ErrorAction SilentlyContinue
if ($existingFiles) {
    Write-Host "    → 기존 파일 정리 ($($existingFiles.Count)개)"
    foreach ($f in $existingFiles) {
        try {
            Remove-Item -Path $f.FullName -Force -ErrorAction Stop
            Write-Host "      - 삭제: $($f.Name)"
        } catch {
            Write-Error "$($f.Name) 삭제 실패 (잠금/권한): $($_.Exception.Message)"
            exit 1
        }
    }
}

# 4c. PS 5.1 Expand-Archive 우회 — .NET 의 ZipFile.ExtractToDirectory 사용.
#     2-arg 버전 (.NET Framework 4.5+ 모두 지원). 4b 에서 dest 비웠으니 overwrite 인자 불필요.
Add-Type -AssemblyName System.IO.Compression.FileSystem -ErrorAction Stop
[System.IO.Compression.ZipFile]::ExtractToDirectory($tmpZip, $INSTALL)

# 4c. 갱신 검증 — 압축 해제 후 xgram.exe 가 실제 갱신됐는지 확인 (silent-skip 차단).
$xgramExe = Join-Path $INSTALL 'xgram.exe'
if (-not (Test-Path $xgramExe)) {
    Write-Error "xgram.exe 가 install dir 에 없음 — 압축 해제 실패 가능. zip 파일 손상 의심."
    exit 1
}
$age = (Get-Date) - (Get-Item $xgramExe).LastWriteTime
if ($age.TotalMinutes -gt 5) {
    Write-Error "xgram.exe 갱신 실패 (LastWriteTime 이 $([int]$age.TotalMinutes)분 전). PowerShell 새로 열고 재시도."
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
Write-Host '다음 단계:'
Write-Host '  xgram init --alias my-laptop      # alias + 패스워드 설정'
Write-Host '  xgram gui                          # Tauri 데스크탑 창'
Write-Host '  xgram daemon                       # foreground 실행 (또는 systemd-task 로 백그라운드)'
Write-Host ''
Write-Host '데모 plan: https://openxgram.org/demo/'
Write-Host ''
