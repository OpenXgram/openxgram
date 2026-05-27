# OpenXgram quickstart - one-line wizard (Windows PowerShell).
#
# Usage:
#   irm https://openxgram.org/quickstart.ps1 | iex
#
# Flow:
#   1. install.ps1 auto-run if xgram missing
#   2. choose [1] new node (seed) or [2] add to existing node (remote daemon login)
#   3. email + GUI password (12+ chars) — for web GUI login
#   4. [1] new node: alias + keystore password -> init -> register (first user -> admin)
#      [2] existing node: remote daemon URL -> email+password login -> JWT saved
#   5. (optional) Discord / Telegram tokens
#   6. save secrets to %USERPROFILE%\.openxgram\.env.ps1
#   7. start daemon + agent (Start-Process Hidden, survives parent exit)
#   8. show status + next commands

# Force UTF-8 — avoid Korean encoding issues on Windows PowerShell 5.1 (cp949 default).
# chcp 65001 + Console.Output/InputEncoding + $OutputEncoding (4 layers).
try {
    $null = & chcp.com 65001 2>&1
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    [Console]::InputEncoding  = [System.Text.Encoding]::UTF8
    $OutputEncoding           = [System.Text.Encoding]::UTF8
    [System.Globalization.CultureInfo]::CurrentCulture = 'ko-KR'
} catch {}

$ErrorActionPreference = 'Stop'

$DataDir  = if ($env:XGRAM_DATA_DIR) { $env:XGRAM_DATA_DIR } else { Join-Path $env:USERPROFILE '.openxgram' }
$EnvFile  = Join-Path $DataDir '.env.ps1'
$Manifest = Join-Path $DataDir 'install-manifest.json'
$DaemonGuiPort = if ($env:XGRAM_DAEMON_GUI_PORT) { $env:XGRAM_DAEMON_GUI_PORT } else { '47302' }

Write-Host ''
Write-Host '═══════════════════════════════════════════════════' -ForegroundColor Cyan
Write-Host '  OpenXgram quickstart' -ForegroundColor Cyan
Write-Host "  data dir: $DataDir"
Write-Host '═══════════════════════════════════════════════════' -ForegroundColor Cyan
Write-Host ''

# 1. Auto-run install.ps1 if xgram missing
$xgramCmd = Get-Command xgram -ErrorAction SilentlyContinue
if (-not $xgramCmd) {
    Write-Host '-> xgram missing — auto-running install.ps1' -ForegroundColor Yellow
    Invoke-Expression (Invoke-RestMethod -UseBasicParsing -Uri 'https://openxgram.org/install.ps1')
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $xgramCmd = Get-Command xgram -ErrorAction SilentlyContinue
    if (-not $xgramCmd) {
        Write-Error 'xgram installed but not in PATH — restart PowerShell and retry'
        return
    }
}
$xgramVersion = & xgram --version 2>&1 | Select-Object -First 1
Write-Host "  xgram : $xgramVersion"
Write-Host ''

# 2. Choose machine mode (only if no manifest)
$mode = '1'
if (-not (Test-Path $Manifest)) {
    Write-Host '── How will you use this machine? ──'
    Write-Host '  [1] New node (fresh seed, independent identity + memory)'
    Write-Host '  [2] Add to existing node (use remote daemon with other machine account)'
    Write-Host ''
    $mode = Read-Host 'Select [1/2] (Enter = 1)'
    if ([string]::IsNullOrWhiteSpace($mode)) { $mode = '1' }
    Write-Host ''
}

# 3. Email + GUI password (both modes)
$email = Read-Host 'Email'
while ($true) {
    $secureGui = Read-Host 'Web GUI password (min 12 chars)' -AsSecureString
    $guiPassword = [System.Net.NetworkCredential]::new('', $secureGui).Password
    if ($guiPassword.Length -ge 12) { break }
    Write-Host '  ✗ Need at least 12 chars — try again' -ForegroundColor Red
}
Write-Host ''

# ── [2] Add to existing node ──
if ($mode -eq '2') {
    $remoteUrl = Read-Host 'Remote daemon URL (e.g. https://other-machine.tailXXXX.ts.net)'
    $remoteUrl = $remoteUrl.TrimEnd('/')
    if ([string]::IsNullOrWhiteSpace($remoteUrl)) {
        Write-Error 'URL required'
        return
    }

    $loginBody = @{ email = $email; password = $guiPassword } | ConvertTo-Json -Compress
    try {
        $loginResp = Invoke-RestMethod -Method Post -Uri "$remoteUrl/v1/auth/login" `
            -ContentType 'application/json' -Body $loginBody
    } catch {
        Write-Error "Remote login failed: $($_.Exception.Message)"
        return
    }
    if (-not $loginResp.jwt_token) {
        Write-Error "JWT missing in response"
        return
    }

    if (-not (Test-Path $DataDir)) { New-Item -ItemType Directory -Path $DataDir -Force | Out-Null }
    $envLines = @(
        '# OpenXgram quickstart (remote node mode)',
        "`$env:XGRAM_DAEMON_URL = '$remoteUrl'",
        "`$env:XGRAM_GUI_JWT = '$($loginResp.jwt_token)'"
    )
    Set-Content -Path $EnvFile -Value $envLines -Encoding UTF8
    Write-Host ''
    Write-Host '✓ Connected to remote node — this machine xgram CLI/GUI uses remote daemon' -ForegroundColor Green
    Write-Host '  (no local daemon on this machine)'
    Write-Host ''
    Write-Host 'Next:'
    Write-Host "  Open web GUI: $remoteUrl/gui/ (or remote Tailscale Funnel URL)"
    Write-Host '  -> log in from anywhere with the same email/password'
    return
}

# ── [1] New node (default) ──
# 4. xgram init — if no manifest, prompt alias + password then init
$alias = ''
if (-not (Test-Path $Manifest)) {
    $defaultAlias = $env:COMPUTERNAME
    $alias = Read-Host "Machine alias (default: $defaultAlias)"
    if ([string]::IsNullOrWhiteSpace($alias)) { $alias = $defaultAlias }

    while ($true) {
        $secure = Read-Host 'keystore password (min 12 chars)' -AsSecureString
        $password = [System.Net.NetworkCredential]::new('', $secure).Password
        if ($password.Length -ge 12) { break }
        Write-Host '  ✗ Need at least 12 chars — try again' -ForegroundColor Red
    }

    $env:XGRAM_KEYSTORE_PASSWORD = $password
    $env:XGRAM_INIT_SKIP_SEED_BACKUP_CONFIRM = '1'

    Write-Host ''
    Write-Host "→ xgram init --alias '$alias'"
    & xgram init --alias $alias
    if ($LASTEXITCODE -ne 0) {
        Write-Error "xgram init failed (exit $LASTEXITCODE)"
        return
    }
} else {
    Write-Host '-> existing install-manifest found — skipping init'
    $existingAlias = (Get-Content $Manifest -Raw | ConvertFrom-Json).machine.alias
    $alias = $existingAlias
    Write-Host "  existing alias: $existingAlias"
    $secure = Read-Host 'keystore password (for saved bots)' -AsSecureString
    $password = [System.Net.NetworkCredential]::new('', $secure).Password
    $env:XGRAM_KEYSTORE_PASSWORD = $password
}
Write-Host ''

# 5. External channels — Enter to skip
Write-Host '── External channels (all optional — Enter to skip) ──'
$discordWebhook   = Read-Host 'Discord webhook URL'
$discordBotToken  = Read-Host 'Discord bot token'
$discordChannelId = Read-Host 'Discord channel id'
$telegramBotToken = Read-Host 'Telegram bot token'
$telegramChatId   = Read-Host 'Telegram chat id'
$anthropicApiKey  = ''
Write-Host ''

# 6. Save .env.ps1
if (-not (Test-Path $DataDir)) { New-Item -ItemType Directory -Path $DataDir -Force | Out-Null }
$envLines = @(
    '# OpenXgram quickstart — restore env in future sessions: . ' + $EnvFile + '',
    "`$env:XGRAM_KEYSTORE_PASSWORD = '$password'"
)
if ($discordWebhook)   { $envLines += "`$env:XGRAM_DISCORD_WEBHOOK_URL = '$discordWebhook'" }
if ($discordBotToken)  { $envLines += "`$env:XGRAM_DISCORD_BOT_TOKEN = '$discordBotToken'" }
if ($discordChannelId) { $envLines += "`$env:XGRAM_DISCORD_CHANNEL_ID = '$discordChannelId'" }
if ($telegramBotToken) { $envLines += "`$env:XGRAM_TELEGRAM_BOT_TOKEN = '$telegramBotToken'" }
if ($telegramChatId)   { $envLines += "`$env:XGRAM_TELEGRAM_CHAT_ID = '$telegramChatId'" }
if ($anthropicApiKey)  { $envLines += "`$env:XGRAM_ANTHROPIC_API_KEY = '$anthropicApiKey'" }
Set-Content -Path $EnvFile -Value $envLines -Encoding UTF8
Write-Host "-> secrets saved: $EnvFile"
Write-Host ''

# 7. Kill existing running processes
Get-Process -Name xgram -ErrorAction SilentlyContinue | ForEach-Object {
    $cmdline = (Get-CimInstance Win32_Process -Filter "ProcessId=$($_.Id)" -ErrorAction SilentlyContinue).CommandLine
    if ($cmdline -match 'daemon|agent') {
        Write-Host "  -> killing existing process: PID $($_.Id) ($cmdline)"
        Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    }
}
Start-Sleep -Milliseconds 500

# 8. daemon — Start-Process Hidden
Write-Host '-> starting daemon'
$daemonLog = Join-Path $DataDir 'daemon.log'
$daemonProc = Start-Process -FilePath (Get-Command xgram).Source `
    -ArgumentList 'daemon' `
    -WindowStyle Hidden `
    -RedirectStandardOutput $daemonLog `
    -RedirectStandardError "$daemonLog.err" `
    -PassThru
Start-Sleep -Seconds 3

# 9. Web GUI user registration (email+password)
Write-Host '-> registering web GUI user'
$regBody = @{ email = $email; password = $guiPassword; alias = $alias } | ConvertTo-Json -Compress
try {
    $regResp = Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$DaemonGuiPort/v1/auth/register" `
        -ContentType 'application/json' -Body $regBody
    Write-Host "  ✓ registered (role=$($regResp.role))" -ForegroundColor Green
} catch {
    $msg = $_.Exception.Message
    if ($msg -match 'already' -or $msg -match 'Already') {
        Write-Host '  -> email already registered — keeping existing account'
    } else {
        Write-Host "  ✗ registration failed: $msg" -ForegroundColor Yellow
        Write-Host "  daemon log: $daemonLog"
    }
}
Write-Host ''

# 10. agent — only if Discord/Telegram channel tokens present
$hasChannel = $discordWebhook -or $telegramBotToken
$agentProc = $null
if ($hasChannel) {
    Write-Host '-> starting agent (external channel forward)'
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

# 11. Status check
Write-Host ''
Write-Host '── running status ──'
$daemonAlive = $daemonProc -and -not $daemonProc.HasExited
$agentAlive  = $agentProc  -and -not $agentProc.HasExited
if ($daemonAlive) {
    Write-Host "  ✓ daemon  PID $($daemonProc.Id)  (log: $daemonLog)" -ForegroundColor Green
} else {
    Write-Host "  ✗ daemon  not running — check log: $daemonLog" -ForegroundColor Red
}
if ($agentProc) {
    if ($agentAlive) {
        Write-Host "  ✓ agent   PID $($agentProc.Id)  (log: $agentLog)" -ForegroundColor Green
    } else {
        Write-Host "  ✗ agent   not running — check log: $agentLog" -ForegroundColor Red
    }
} else {
    Write-Host '  - agent   not running (no external channel token — skipped)'
}
Write-Host ''

# 12. Guide
Write-Host '═══════════════════════════════════════════════════' -ForegroundColor Cyan
Write-Host '  ✓ OpenXgram running' -ForegroundColor Green
Write-Host ''
Write-Host '  Web GUI login:'
Write-Host "    email    : $email"
Write-Host '    password : (the GUI password you entered)'
Write-Host ''
Write-Host '  Next commands:'
Write-Host '    xgram peer send --alias <target> --body "message"   # send message'
Write-Host '    xgram bot register <name>                            # register another bot'
Write-Host '    xgram gui                                            # web GUI (auto-opens Tailscale Funnel URL)'
Write-Host ''
Write-Host '  Restore env (new PowerShell window):'
Write-Host "    . '$EnvFile'"
Write-Host '═══════════════════════════════════════════════════' -ForegroundColor Cyan
