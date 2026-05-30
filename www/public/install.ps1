# OpenXgram installer - Windows PowerShell.
#
# Usage (PowerShell):
#   irm https://openxgram.org/install.ps1 | iex
#   $env:OPENXGRAM_VERSION="v0.2.0-rc.15"; irm https://openxgram.org/install.ps1 | iex
#
# Privacy: GitHub Releases asset download + SHA256 verify, no telemetry.

# Force UTF-8 — avoid Korean encoding issues on Windows PowerShell 5.1 (cp949 default).
# chcp 65001 + Console.Output/InputEncoding + $OutputEncoding (4 layers).
try {
    $null = & chcp.com 65001 2>&1
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
    [Console]::InputEncoding  = [System.Text.Encoding]::UTF8
    $OutputEncoding           = [System.Text.Encoding]::UTF8
} catch {
    # Encoding setup failure doesn't block install — only messages may be garbled.
}

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
    Write-Host "==> Step 1: query latest tag — $api"
    try {
        $rel = Invoke-RestMethod -UseBasicParsing -Uri $api
        $tag = $rel.tag_name
    } catch {
        # latest API may filter prereleases — try first from list
        $rels = Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$REPO/releases"
        $tag = $rels[0].tag_name
    }
} else {
    $tag = $VERSION
}
Write-Host "    → tag = $tag"

# 2. Build download URL
$asset   = "xgram-$tag-x86_64-windows.zip"
$dlUrl   = "https://github.com/$REPO/releases/download/$tag/$asset"
$shaUrl  = "$dlUrl.sha256"
$tmpZip  = Join-Path $env:TEMP $asset
$tmpSha  = "$tmpZip.sha256"

Write-Host "==> Step 2: download — $dlUrl"
Invoke-WebRequest -UseBasicParsing -Uri $dlUrl -OutFile $tmpZip
Invoke-WebRequest -UseBasicParsing -Uri $shaUrl -OutFile $tmpSha

# 3. SHA256 verify
Write-Host '==> Step 3: SHA256 verify'
$expected = (Get-Content $tmpSha).Split(' ')[0].ToLower()
$actual   = (Get-FileHash $tmpZip -Algorithm SHA256).Hash.ToLower()
if ($expected -ne $actual) {
    Write-Error "SHA256 mismatch — expected $expected / actual $actual"
    exit 1
}
Write-Host "    ✓ SHA256 ok ($actual.Substring(0, 12)...)"

# 4. Prepare install dir + extract
Write-Host "==> Step 4: install → $INSTALL"
if (-not (Test-Path $INSTALL)) {
    New-Item -ItemType Directory -Force -Path $INSTALL | Out-Null
}

# 4a-pre. Stop scheduled tasks + services that respawn xgram.exe (rc.166+).
#         이름 모름 — *xgram* / *OpenXgram* glob 매칭하는 모든 task/service 자동 정지.
#         재시작은 Step 7 의 마지막에 자동.
$stoppedTasks = @()
$stoppedSvcs  = @()
try {
    Get-ScheduledTask -ErrorAction SilentlyContinue |
        Where-Object { $_.TaskName -like "*xgram*" -or $_.TaskName -like "*OpenXgram*" } |
        ForEach-Object {
            Write-Host "    -> stop scheduled task: $($_.TaskName)"
            schtasks /End /TN $_.TaskName 2>$null | Out-Null
            $script:stoppedTasks += $_.TaskName
        }
} catch {}
try {
    Get-Service -ErrorAction SilentlyContinue |
        Where-Object { ($_.Name -like "*xgram*" -or $_.Name -like "*OpenXgram*") -and $_.Status -eq 'Running' } |
        ForEach-Object {
            Write-Host "    -> stop service: $($_.Name)"
            Stop-Service -Name $_.Name -Force -ErrorAction SilentlyContinue
            $script:stoppedSvcs += $_.Name
        }
} catch {}

# rc.174+: OpenXgram 표준 port (47300 transport, 47302 GUI, 47301 MCP, 7300 legacy transport, 7302 legacy GUI) 점유 process 자동 kill.
#          port 충돌 시 새 daemon 이 bind fail → 부분 작동 → process_inbound 도 작동 안 함 (실제 발견 사례).
foreach ($port in 47300, 47302, 47301, 7300, 7302) {
    try {
        $conn = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction SilentlyContinue
        if ($conn) {
            $procIds = @($conn.OwningProcess | Sort-Object -Unique)
            foreach ($procId in $procIds) {
                if ($procId -and $procId -gt 0) {
                    $proc = Get-Process -Id $procId -ErrorAction SilentlyContinue
                    if ($proc) {
                        Write-Host "    -> kill port $port owner: $($proc.Name) PID=$procId"
                        Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue
                    }
                }
            }
        }
    } catch {}
}
Start-Sleep -Milliseconds 500

# 4a. Locked .exe causes silent skip — kill running processes first.
# v0.2.0-rc.24+: xgram-desktop deprecated (Tauri -> web GUI) — only check xgram.
# rc.166+: 최대 5회 재시도 (respawn race 방지).
for ($i = 0; $i -lt 5; $i++) {
    $running = Get-Process -Name xgram -ErrorAction SilentlyContinue
    if (-not $running) { break }
    if ($i -eq 0) { Write-Host "    -> killing running OpenXgram processes for update (no reboot)" }
    foreach ($p in $running) {
        Write-Host "      - $($p.Name) (PID $($p.Id))"
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Milliseconds 400
}

# 4b. Delete & recreate install dir — avoids all PS 5.1 silent-skip cases.
#     Bypasses edge cases (hidden, ACL, per-file lock).
# rc.166+: respawn race 대비 5회 재시도.
Write-Host "    -> cleaning install dir: $INSTALL"
if (Test-Path $INSTALL) {
    $deleted = $false
    for ($i = 0; $i -lt 5; $i++) {
        try {
            Remove-Item -Path $INSTALL -Recurse -Force -ErrorAction Stop
            $deleted = $true
            break
        } catch {
            # 다시 kill (또 누군가 띄웠을 수 있음) + 짧게 대기 후 retry.
            Get-Process -Name xgram -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
            Start-Sleep -Seconds 1
        }
    }
    if (-not $deleted) {
        Write-Error "install dir delete failed after 5 retries (lock/perm)."
        Write-Error "Kill manually then retry: Get-Process xgram | Stop-Process -Force"
        exit 1
    }
}
New-Item -ItemType Directory -Force -Path $INSTALL | Out-Null

# 4c. Extract into fresh empty dir.
Expand-Archive -Path $tmpZip -DestinationPath $INSTALL -Force

# 4c-1. Log extract result (debugging — catches silent skip).
Write-Host "    -> install dir contents (after extract):"
Get-ChildItem $INSTALL -File | ForEach-Object {
    Write-Host "      - $($_.Name)  $([int]($_.Length/1024))KB  $($_.LastWriteTime)"
}

# 4d. Verify — only check xgram.exe exists. LastWriteTime (zip vs local time)
#     timezone false alarms; step 4b empties dir so silent-skip impossible.
$xgramExe = Join-Path $INSTALL 'xgram.exe'
if (-not (Test-Path $xgramExe)) {
    Write-Error "xgram.exe missing in install dir — extract failed. zip may be corrupt."
    exit 1
}

Remove-Item $tmpZip, $tmpSha -ErrorAction SilentlyContinue

# 5. Add to PATH permanently (User scope, skip if present)
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$INSTALL*") {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$INSTALL", 'User')
    Write-Host "    ✓ added to PATH ($INSTALL) — new PowerShell windows pick it up"
} else {
    Write-Host "    (already in PATH)"
}
$env:Path += ";$INSTALL"

# 6. Verify
Write-Host ''
Write-Host '==> install complete' -ForegroundColor Green
& "$INSTALL\xgram.exe" --version

# 7. (optional) auto-start daemon/agent if existing manifest + password env exist.
#    One-line restore for users who already ran quickstart.ps1 wizard.
$dataDir = Join-Path $env:USERPROFILE '.openxgram'
$manifestPath = Join-Path $dataDir 'install-manifest.json'
if ((Test-Path $manifestPath) -and $env:XGRAM_KEYSTORE_PASSWORD) {
    Write-Host ''
    Write-Host '==> existing install detected — auto-starting daemon' -ForegroundColor Cyan

    # Kill existing xgram daemon / agent if any
    Get-Process -Name xgram -ErrorAction SilentlyContinue | ForEach-Object {
        $cmdline = (Get-CimInstance Win32_Process -Filter "ProcessId=$($_.Id)" -ErrorAction SilentlyContinue).CommandLine
        if ($cmdline -match 'daemon|agent') {
            Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
        }
    }
    Start-Sleep -Milliseconds 500

    # daemon
    $daemonLog = Join-Path $dataDir 'daemon.log'
    $daemonProc = Start-Process -FilePath "$INSTALL\xgram.exe" `
        -ArgumentList 'daemon' `
        -WindowStyle Hidden `
        -RedirectStandardOutput $daemonLog `
        -RedirectStandardError "$daemonLog.err" `
        -PassThru
    Start-Sleep -Seconds 1
    if ($daemonProc -and -not $daemonProc.HasExited) {
        Write-Host "    ✓ daemon PID $($daemonProc.Id)  (log: $daemonLog)"
    } else {
        Write-Host "    ⚠ daemon not running — check logs then run xgram daemon manually" -ForegroundColor Yellow
    }

    # agent (try to extract Discord/Telegram tokens from vault)
    $agentArgs = @('agent')
    try {
        $discordWebhook = & "$INSTALL\xgram.exe" vault get notify.discord.webhook_url 2>$null
        if ($LASTEXITCODE -eq 0 -and $discordWebhook) {
            $agentArgs += @('--discord-webhook-url', $discordWebhook.Trim())
        }
    } catch {}
    try {
        $discordBotToken = & "$INSTALL\xgram.exe" vault get notify.discord.bot_token 2>$null
        if ($LASTEXITCODE -eq 0 -and $discordBotToken) {
            $agentArgs += @('--discord-bot-token', $discordBotToken.Trim())
        }
    } catch {}
    try {
        $discordChannelId = & "$INSTALL\xgram.exe" vault get notify.discord.channel_id 2>$null
        if ($LASTEXITCODE -eq 0 -and $discordChannelId) {
            $agentArgs += @('--discord-channel-id', $discordChannelId.Trim())
        }
    } catch {}

    if ($agentArgs.Count -gt 1) {
        $agentLog = Join-Path $dataDir 'agent.log'
        $agentProc = Start-Process -FilePath "$INSTALL\xgram.exe" `
            -ArgumentList $agentArgs `
            -WindowStyle Hidden `
            -RedirectStandardOutput $agentLog `
            -RedirectStandardError "$agentLog.err" `
            -PassThru
        Start-Sleep -Seconds 1
        if ($agentProc -and -not $agentProc.HasExited) {
            Write-Host "    ✓ agent PID $($agentProc.Id)  (Discord/Telegram forward active)"
        }
    } else {
        Write-Host "    (agent not running — no Discord/Telegram token. Run xgram setup discord then restart)"
    }
}

# 8. Restart any scheduled tasks / services we stopped in Step 4a-pre.
#    rc.166+: 자동화 마무리 — 사용자가 schtasks/nssm 따로 안 건드려도 됨.
if ($stoppedTasks.Count -gt 0 -or $stoppedSvcs.Count -gt 0) {
    Write-Host ''
    Write-Host '==> Step 8: restart stopped tasks/services' -ForegroundColor Cyan
    foreach ($t in $stoppedTasks) {
        Write-Host "    -> start scheduled task: $t"
        schtasks /Run /TN $t 2>$null | Out-Null
    }
    foreach ($s in $stoppedSvcs) {
        Write-Host "    -> start service: $s"
        Start-Service -Name $s -ErrorAction SilentlyContinue
    }
}


# 8.5 (rc.174+, updated rc.182) Auto-register Scheduled Task 'OpenXgram-Daemon' (ONLOGON) + auto-restart on exit.
#      ONLOGON trigger + REPEAT every 1 min for indefinite duration + RestartCount=999 if task fails.
#      rc.182: 핵심 변경 — auto restart on daemon process exit (이전엔 process 죽으면 dead).
#      Wrapper script (.cmd) 가 무한 loop 으로 daemon 실행 → 죽으면 5초 후 재시작.
$daemonTaskName = 'OpenXgram-Daemon'
Write-Host ''
Write-Host '==> Step 8.5: register OpenXgram-Daemon Scheduled Task (auto-start + auto-restart)' -ForegroundColor Cyan
$dataDir = Join-Path $env:USERPROFILE '.openxgram'
$daemonLog = Join-Path $dataDir 'daemon.log'
$wrapperPath = Join-Path $INSTALL 'openxgram-daemon-wrapper.cmd'

# Wrapper .cmd: infinite loop. daemon process 가 exit 하면 5초 후 재시작 (kernel signal trap 같이).
# Windows 의 Scheduled Task 의 restart-on-fail 보다 robust (exit code 0 이여도 restart).
$wrapperContent = @"
@echo off
:loop
echo [%DATE% %TIME%] starting openxgram daemon >> "$daemonLog" 2>&1
REM rc.184: --bind 0.0.0.0 external access. GUI port 는 daemon 자체 fallback (47302 fail 시 17302/47312/27302/random 자동).
"$INSTALL\xgram.exe" daemon --bind 0.0.0.0:47300 --gui-bind 0.0.0.0:47302 >> "$daemonLog" 2>&1
echo [%DATE% %TIME%] daemon exited code=%ERRORLEVEL%, restart in 5s >> "$daemonLog" 2>&1
timeout /t 5 /nobreak > nul
goto loop
"@
try {
    Set-Content -Path $wrapperPath -Value $wrapperContent -Encoding ASCII -Force
    Write-Host "    [OK] wrapper script: $wrapperPath"
} catch {
    Write-Host "    [WARN] wrapper script write failed: $($_.Exception.Message)" -ForegroundColor Yellow
}

$action  = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument "/c `"$wrapperPath`"" -WorkingDirectory $INSTALL
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1)
# rc.183: ExecutionTimeLimit 제거 → default (PT72H or 무제한) 사용. P36500D 가 Windows max 초과로 등록 fail 한 버그 수정.
$settings.ExecutionTimeLimit = 'PT0S'  # PT0S = no limit (Windows Task Scheduler 의 무제한 표현)
$principal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive
try {
    Register-ScheduledTask -TaskName $daemonTaskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Description 'OpenXgram sidecar daemon (auto-start on logon + infinite restart loop)' -Force | Out-Null
    Write-Host "    [OK] Scheduled Task '$daemonTaskName' registered (auto-start + auto-restart loop)"
    Start-ScheduledTask -TaskName $daemonTaskName -ErrorAction SilentlyContinue
} catch {
    Write-Host "    [WARN] Scheduled Task register failed: $($_.Exception.Message)" -ForegroundColor Yellow
}

# 8.6 (rc.174+) WSL warm-up on logon (if wsl.exe available).
#      WSL2 vmcompute/LxssManager auto-starts at boot; first distro init is lazy.
#      `wsl --exec /bin/true` triggers warm-up so Linux env is ready when user logs in.
$wslAvailable = Get-Command wsl.exe -ErrorAction SilentlyContinue
if ($wslAvailable) {
    $wslTaskName = 'OpenXgram-WSL-Boot'
    $existingWsl = Get-ScheduledTask -TaskName $wslTaskName -ErrorAction SilentlyContinue
    if (-not $existingWsl) {
        Write-Host ''
        Write-Host '==> Step 8.6: register OpenXgram-WSL-Boot Scheduled Task (WSL warm-up on logon)' -ForegroundColor Cyan
        try {
            $wslAction  = New-ScheduledTaskAction -Execute 'wsl.exe' -Argument '--exec /bin/true'
            $wslTrigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
            $wslSettings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 5)
            $wslPrincipal = New-ScheduledTaskPrincipal -UserId $env:USERNAME -LogonType Interactive
            Register-ScheduledTask -TaskName $wslTaskName -Action $wslAction -Trigger $wslTrigger -Settings $wslSettings -Principal $wslPrincipal -Description 'WSL warm-up (start default distro on logon)' -Force | Out-Null
            Write-Host "    [OK] Scheduled Task '$wslTaskName' registered (WSL warm-up on logon)"
        } catch {
            Write-Host "    [WARN] WSL Scheduled Task register failed: $($_.Exception.Message)" -ForegroundColor Yellow
        }
    } else {
        Write-Host "    (Scheduled Task '$wslTaskName' already exists)" -ForegroundColor DarkGray
    }
}

# 9. Auto MCP + identity + SessionStart hook (rc.169+).
#    Claude Code 가 깔려있으면 (~/.claude.json 존재) 자동으로 mcp-install --full 실행.
#    → 새 Claude Code 세션마다 openxgram MCP 도구 + Identity block + 가이드 자동 인식.
$claudeJson = Join-Path $env:USERPROFILE '.claude.json'
if (Test-Path $claudeJson) {
    Write-Host ''
    Write-Host '==> Step 9: auto MCP + identity + SessionStart hook' -ForegroundColor Cyan
    try {
        & "$INSTALL\xgram.exe" mcp-install --scope user --full --use-path-lookup 2>&1 | ForEach-Object { Write-Host "    $_" }
        Write-Host '    [OK] New Claude Code sessions will auto-recognize openxgram MCP + identity + guide'
    } catch {
        Write-Host "    [WARN] mcp-install failed: $($_.Exception.Message)" -ForegroundColor Yellow
        Write-Host '    Manual: xgram mcp-install --scope user --full --use-path-lookup'
    }
} else {
    Write-Host ''
    Write-Host '    (Claude Code not installed - Step 9 skipped. After install: xgram mcp-install --scope user --full --use-path-lookup)' -ForegroundColor DarkGray
}

Write-Host ''
Write-Host 'Next steps:' -ForegroundColor Cyan
Write-Host ''
Write-Host '[1] Initialize identity (one-time):'
Write-Host '    xgram init --alias my-laptop'
Write-Host ''
Write-Host '[2] (optional) Connect Discord / Telegram — interactive wizard:'
Write-Host '    xgram setup discord            # webhook + bot token + channel id'
Write-Host '    xgram setup telegram           # bot token + chat id'
Write-Host ''
Write-Host '[3] Full setup for Claude Code / other LLMs (MCP + identity + SessionStart hook):'
Write-Host '    xgram mcp-install --scope user --full --use-path-lookup'
Write-Host '    # ~/.claude.json (MCP) + ./CLAUDE.md (identity) + ~/.claude/settings.json (hook) at once'
Write-Host '    # -> new Claude Code sessions auto-detect openxgram.* MCP tools'
Write-Host ''
Write-Host '[4] daemon + web GUI (Tailscale Funnel):'
Write-Host '    xgram daemon                   # foreground or background'
Write-Host '    sudo tailscale funnel --bg --https=443 http://localhost:47310'
Write-Host '    xgram gui                      # -> opens browser at https://<machine>.tailXXXX.ts.net'
Write-Host ''
Write-Host 'One-shot full setup (interactive wizard):'
Write-Host '    irm https://openxgram.org/quickstart.ps1 | iex'
Write-Host ''
Write-Host 'Demo plan: https://openxgram.org/demo/'
Write-Host ''
