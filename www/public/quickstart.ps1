# OpenXgram quickstart — 한 줄 마법사 (Windows PowerShell).
#
# 사용:
#   irm https://openxgram.org/quickstart.ps1 | iex
#
# 흐름:
#   1. xgram 미설치면 install.ps1 자동 실행
#   2. alias / keystore 패스워드 입력 (기존 init 있으면 skip)
#   3. Discord webhook / Telegram bot / Anthropic API 키 입력 (Enter 로 skip)
#   4. %USERPROFILE%\.openxgram\.env 에 비밀 저장
#   5. daemon + agent 백그라운드 가동 (Start-Process Hidden, 부모 종료 후 생존)
#   6. 상태 확인 + 다음 명령 안내

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

# 2. init — manifest 없으면 alias + 패스워드 prompt 후 init
if (-not (Test-Path $Manifest)) {
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

# 3. 외부 채널 / LLM 토큰 — Enter 로 skip
Write-Host '── 외부 채널 / LLM 연동 (모두 선택 — Enter 로 skip) ──'
$discordWebhook   = Read-Host 'Discord webhook URL'
$discordBotToken  = Read-Host 'Discord bot token'
$discordChannelId = Read-Host 'Discord channel id'
$telegramBotToken = Read-Host 'Telegram bot token'
$telegramChatId   = Read-Host 'Telegram chat id'
$anthropicApiKey  = Read-Host 'Anthropic API key'
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

# 7. agent — Discord/Telegram/Anthropic 키 있을 때만
$hasChannel = $discordWebhook -or $telegramBotToken -or $anthropicApiKey
$agentProc = $null
if ($hasChannel) {
    Write-Host '→ agent 가동 (외부 채널 forward + LLM 응답)'
    $agentArgs = @('agent')
    if ($discordWebhook)   { $agentArgs += @('--discord-webhook-url', $discordWebhook) }
    if ($discordBotToken)  { $agentArgs += @('--discord-bot-token', $discordBotToken) }
    if ($discordChannelId) { $agentArgs += @('--discord-channel-id', $discordChannelId) }
    if ($anthropicApiKey)  { $agentArgs += @('--anthropic-api-key', $anthropicApiKey) }
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
Write-Host '    xgram gui                                            # 데스크탑 GUI'
Write-Host ''
Write-Host '  환경 복원 (새 PowerShell 창):'
Write-Host "    . '$EnvFile'"
Write-Host '═══════════════════════════════════════════════════' -ForegroundColor Cyan
