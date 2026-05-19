# OpenXgram quickstart - one-line wizard (Windows PowerShell).
#
# Usage:
#   irm https://openxgram.org/quickstart.ps1 | iex
#
# Flow:
#   1. install.ps1 auto-run if xgram missing
#   2. choose [1] new node (seed) or [2] add to existing node (oxg:// URL)
#   3. alias / keystore password / optional Discord-Telegram tokens
#   4. save secrets to %USERPROFILE%\.openxgram\.env.ps1
#   5. start daemon + agent (Start-Process Hidden, survives parent exit)
#   6. show status + next commands

$ErrorActionPreference = 'Stop'

try {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    $OutputEncoding            = [System.Text.Encoding]::UTF8
} catch {}

$DataDir  = if ($env:XGRAM_DATA_DIR) { $env:XGRAM_DATA_DIR } else { Join-Path $env:USERPROFILE '.openxgram' }
$EnvFile  = Join-Path $DataDir '.env.ps1'
$Manifest = Join-Path $DataDir 'install-manifest.json'

Write-Host ''
Write-Host '═══════════════════════════════════════════════════' -ForegroundColor Cyan
Write-Host '  OpenXgram quickstart' -ForegroundColor Cyan
Write-Host "  데이터 디렉토리: $DataDir"
Write-Host '═══════════════════════════════════════════════════' -ForegroundColor Cyan
Write-Host ''

# 1. xgram 미설치면 install.ps1 자동 실행
$xgramCmd = Get-Command xgram -ErrorAction SilentlyContinue
if (-not $xgramCmd) {
    Write-Host '→ xgram 미설치 — install.ps1 자동 실행' -ForegroundColor Yellow
    Invoke-Expression (Invoke-RestMethod -UseBasicParsing -Uri 'https://openxgram.org/install.ps1')
    # 새 PATH 반영
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $xgramCmd = Get-Command xgram -ErrorAction SilentlyContinue
    if (-not $xgramCmd) {
        Write-Error 'xgram 설치 후에도 PATH 에 없음 — PowerShell 재시작 후 다시 실행'
        return
    }
}
$xgramVersion = & xgram --version 2>&1 | Select-Object -First 1
Write-Host "  xgram : $xgramVersion"
Write-Host ''

# 2. init — manifest 없으면 사용자에게 [1] 새 노드 / [2] 기존 노드 추가 선택
if (-not (Test-Path $Manifest)) {
    Write-Host '── 이 머신을 어떻게 사용하시겠어요? ──'
    Write-Host '  [1] 새 노드로 시작 (시드 신규 발급, 독립 신원·메모리)'
    Write-Host '  [2] 기존 노드에 머신 추가 (다른 머신에서 발급한 페어링 URL 사용)'
    Write-Host ''
    $mode = Read-Host '선택 [1/2] (Enter = 1)'
    if ([string]::IsNullOrWhiteSpace($mode)) { $mode = '1' }

    if ($mode -eq '2') {
        # ── 기존 노드 페어링 ──
        Write-Host ''
        Write-Host '기존 노드에서 다음 명령으로 페어링 URL 생성:'
        Write-Host '  xgram pair-desktop'
        Write-Host '→ oxg://alias@host:port#token=xxx 형태 URL 출력'
        Write-Host ''
        $oxgUrl = Read-Host '페어링 URL 입력 (oxg://...)'
        if (-not ($oxgUrl -match '^oxg://')) {
            Write-Error '잘못된 URL — oxg:// 로 시작해야 합니다'
            return
        }
        Write-Host ''
        Write-Host "→ xgram link $oxgUrl"
        & xgram link $oxgUrl
        if ($LASTEXITCODE -ne 0) {
            Write-Error "xgram link 실패 (exit $LASTEXITCODE) — 네트워크/토큰 확인"
            return
        }
        Write-Host ''
        Write-Host '✓ 기존 노드 연결됨 — 이 머신의 xgram CLI/GUI가 원격 daemon 사용' -ForegroundColor Green
        Write-Host '  (이 머신에는 daemon 띄우지 않음, 모든 명령은 원격에서 처리)'
        Write-Host ''
        Write-Host '추가 설정 (Discord/Telegram 등)은 원격 노드에서 진행하세요.'
        return
    }

    # ── [1] 새 노드 (default) ──
    $defaultAlias = $env:COMPUTERNAME
    $alias = Read-Host "이 머신 alias (default: $defaultAlias)"
    if ([string]::IsNullOrWhiteSpace($alias)) { $alias = $defaultAlias }

    while ($true) {
        $secure = Read-Host 'keystore 패스워드 (최소 12자)' -AsSecureString
        $password = [System.Net.NetworkCredential]::new('', $secure).Password
        if ($password.Length -ge 12) { break }
        Write-Host '  ✗ 최소 12자 — 다시 입력' -ForegroundColor Red
    }

    $env:XGRAM_KEYSTORE_PASSWORD = $password
    $env:XGRAM_INIT_SKIP_SEED_BACKUP_CONFIRM = '1'

    Write-Host ''
    Write-Host "→ xgram init --alias '$alias'"
    & xgram init --alias $alias
    if ($LASTEXITCODE -ne 0) {
        Write-Error "xgram init 실패 (exit $LASTEXITCODE)"
        return
    }
} else {
    Write-Host '→ 기존 install-manifest 발견 — init 건너뜀'
    $existingAlias = (Get-Content $Manifest -Raw | ConvertFrom-Json).machine.alias
    Write-Host "  기존 alias: $existingAlias"
    $secure = Read-Host 'keystore 패스워드 입력 (저장된 봇 가동용)' -AsSecureString
    $password = [System.Net.NetworkCredential]::new('', $secure).Password
    $env:XGRAM_KEYSTORE_PASSWORD = $password
}
Write-Host ''

# 3. 외부 채널 — Enter 로 skip. (LLM 키는 wizard 에서 묻지 않음.)
Write-Host '── 외부 채널 (모두 선택 — Enter 로 skip) ──'
$discordWebhook   = Read-Host 'Discord webhook URL'
$discordBotToken  = Read-Host 'Discord bot token'
$discordChannelId = Read-Host 'Discord channel id'
$telegramBotToken = Read-Host 'Telegram bot token'
$telegramChatId   = Read-Host 'Telegram chat id'
$anthropicApiKey  = ''
Write-Host ''

# 4. .env.ps1 저장 (PowerShell 친화 — source 가능)
if (-not (Test-Path $DataDir)) { New-Item -ItemType Directory -Path $DataDir -Force | Out-Null }
$envLines = @(
    '# OpenXgram quickstart — 다음 세션부터 . ' + $EnvFile + ' 로 환경 복원',
    "`$env:XGRAM_KEYSTORE_PASSWORD = '$password'"
)
if ($discordWebhook)   { $envLines += "`$env:XGRAM_DISCORD_WEBHOOK_URL = '$discordWebhook'" }
if ($discordBotToken)  { $envLines += "`$env:XGRAM_DISCORD_BOT_TOKEN = '$discordBotToken'" }
if ($discordChannelId) { $envLines += "`$env:XGRAM_DISCORD_CHANNEL_ID = '$discordChannelId'" }
if ($telegramBotToken) { $envLines += "`$env:XGRAM_TELEGRAM_BOT_TOKEN = '$telegramBotToken'" }
if ($telegramChatId)   { $envLines += "`$env:XGRAM_TELEGRAM_CHAT_ID = '$telegramChatId'" }
if ($anthropicApiKey)  { $envLines += "`$env:XGRAM_ANTHROPIC_API_KEY = '$anthropicApiKey'" }
Set-Content -Path $EnvFile -Value $envLines -Encoding UTF8
Write-Host "→ 비밀 저장: $EnvFile"
Write-Host ''

# 5. 기존 가동 프로세스 종료
Get-Process -Name xgram -ErrorAction SilentlyContinue | ForEach-Object {
    $cmdline = (Get-CimInstance Win32_Process -Filter "ProcessId=$($_.Id)" -ErrorAction SilentlyContinue).CommandLine
    if ($cmdline -match 'daemon|agent') {
        Write-Host "  → 기존 프로세스 종료: PID $($_.Id) ($cmdline)"
        Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    }
}
Start-Sleep -Milliseconds 500

# 6. daemon — Start-Process Hidden (Windows 에서 detach)
Write-Host '→ daemon 가동'
$daemonLog = Join-Path $DataDir 'daemon.log'
$daemonProc = Start-Process -FilePath (Get-Command xgram).Source `
    -ArgumentList 'daemon' `
    -WindowStyle Hidden `
    -RedirectStandardOutput $daemonLog `
    -RedirectStandardError "$daemonLog.err" `
    -PassThru
Start-Sleep -Seconds 2

# 7. agent — Discord/Telegram 채널 토큰 있을 때만 가동 (forward 만)
$hasChannel = $discordWebhook -or $telegramBotToken
$agentProc = $null
if ($hasChannel) {
    Write-Host '→ agent 가동 (외부 채널 forward)'
    $agentArgs = @('agent')
    if ($discordWebhook)   { $agentArgs += @('--discord-webhook-url', $discordWebhook) }
    if ($discordBotToken)  { $agentArgs += @('--discord-bot-token', $discordBotToken) }
    if ($discordChannelId) { $agentArgs += @('--discord-channel-id', $discordChannelId) }
    $agentLog = Join-Path $DataDir 'agent.log'
    $agentProc = Start-Process -FilePath (Get-Command xgram).Source `
        -ArgumentList $agentArgs `
        -WindowStyle Hidden `
        -RedirectStandardOutput $agentLog `
        -RedirectStandardError "$agentLog.err" `
        -PassThru
    Start-Sleep -Seconds 2
}

# 8. 상태 확인
Write-Host ''
Write-Host '── 가동 상태 ──'
$daemonAlive = $daemonProc -and -not $daemonProc.HasExited
$agentAlive  = $agentProc  -and -not $agentProc.HasExited
if ($daemonAlive) {
    Write-Host "  ✓ daemon  PID $($daemonProc.Id)  (log: $daemonLog)" -ForegroundColor Green
} else {
    Write-Host "  ✗ daemon  미가동 — 로그 확인: $daemonLog" -ForegroundColor Red
}
if ($agentProc) {
    if ($agentAlive) {
        Write-Host "  ✓ agent   PID $($agentProc.Id)  (log: $agentLog)" -ForegroundColor Green
    } else {
        Write-Host "  ✗ agent   미가동 — 로그 확인: $agentLog" -ForegroundColor Red
    }
} else {
    Write-Host '  - agent   미가동 (외부 채널/LLM 토큰 없어 skip 됐을 수 있음)'
}
Write-Host ''

# 9. 안내
Write-Host '═══════════════════════════════════════════════════' -ForegroundColor Cyan
Write-Host '  ✓ OpenXgram 가동 완료' -ForegroundColor Green
Write-Host ''
Write-Host '  다음 명령:'
Write-Host '    xgram peer send --alias <대상> --body "메시지"   # 메시지 보내기'
Write-Host '    xgram bot register <name>                            # 추가 봇 등록'
Write-Host '    xgram pair-desktop                                   # 다른 머신 페어링 URL'
Write-Host '    xgram gui                                            # 웹 GUI (Tailscale Funnel URL 자동 오픈)'
Write-Host ''
Write-Host '  환경 복원 (새 PowerShell 창):'
Write-Host "    . '$EnvFile'"
Write-Host '═══════════════════════════════════════════════════' -ForegroundColor Cyan
