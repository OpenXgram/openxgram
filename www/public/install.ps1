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

# rc.186 patch: admin 권한 자동 elevate. process kill 가 admin 필요한 케이스 대응.
# UAC prompt 한 번만 → 모든 kill/Register-ScheduledTask admin 권한으로 실행.
$isAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    Write-Host '==> Re-launching as Administrator (UAC prompt)...' -ForegroundColor Cyan
    $tempScript = Join-Path $env:TEMP ("openxgram-install-" + [guid]::NewGuid() + ".ps1")
    try {
        Invoke-WebRequest -UseBasicParsing -Uri 'https://openxgram.org/install.ps1' -OutFile $tempScript
        $envPrefix = ''
        if ($env:OPENXGRAM_VERSION) { $envPrefix = "`$env:OPENXGRAM_VERSION='$env:OPENXGRAM_VERSION'; " }
        $cmd = "$envPrefix & '$tempScript'; Write-Host ''; Write-Host 'Done. Press Enter to close.'; Read-Host"
        Start-Process powershell -Verb RunAs -ArgumentList '-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', $cmd | Out-Null
        Write-Host '   (continuing in elevated window — this window can be closed)' -ForegroundColor DarkGray
        exit 0
    } catch {
        Write-Host "    [WARN] elevate 실패 ($($_.Exception.Message)) — non-admin 으로 계속 (일부 step 가 fail 할 수 있음)" -ForegroundColor Yellow
    }
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
# rc.186: unique tmp file (timestamp suffix) — Windows Defender / 옛 zip lock 으로 download fail 회피.
$tsSuffix = (Get-Date -Format 'yyyyMMddHHmmss')
$tmpZip   = Join-Path $env:TEMP "xgram-${tsSuffix}-${asset}"
$tmpSha   = "$tmpZip.sha256"

# 옛 tmp zip 정리 (file lock 안 잡힌 것만)
Get-ChildItem -Path $env:TEMP -Filter "xgram-*-${asset}" -ErrorAction SilentlyContinue | ForEach-Object {
    try { Remove-Item $_.FullName -Force -ErrorAction Stop } catch { Write-Host "    (skip locked old zip: $($_.Name))" -ForegroundColor DarkGray }
}

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
            Write-Host "    -> stop + disable scheduled task: $($_.TaskName)"
            # rc.186 patch: stop + disable. /End 만 으로는 5초 후 wrapper restart 가 새 process spawn → kill race.
            # Disable 가 task 자체 비활성 → spawn 안 함. Step 8.5 후 다시 Enable.
            schtasks /End /TN $_.TaskName 2>$null | Out-Null
            Disable-ScheduledTask -TaskName $_.TaskName -ErrorAction SilentlyContinue | Out-Null
            $script:stoppedTasks += $_.TaskName
        }
} catch {}
# wrapper.cmd 의 cmd.exe + xgram.exe descendants 모두 kill (Scheduled Task 가 disable 된 후라 안 spawn).
try {
    $procs = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object { ($_.Name -eq 'cmd.exe' -and $_.CommandLine -match 'openxgram-daemon-wrapper') -or $_.Name -eq 'xgram.exe' }
    foreach ($p in $procs) {
        Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue
    }
} catch {}
Start-Sleep -Seconds 2
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
# rc.186+: wrapper.cmd 의 cmd.exe parent + xgram.exe child 둘 다 kill.
for ($i = 0; $i -lt 5; $i++) {
    $running = Get-Process -Name xgram -ErrorAction SilentlyContinue
    # cmd.exe 중 openxgram-daemon-wrapper 실행 중인 것도 찾음
    $wrapperCmds = @()
    try {
        $wrapperCmds = Get-CimInstance Win32_Process -Filter "Name='cmd.exe'" -ErrorAction SilentlyContinue |
            Where-Object { $_.CommandLine -and $_.CommandLine -match 'openxgram-daemon-wrapper' }
    } catch {}
    if (-not $running -and (-not $wrapperCmds -or $wrapperCmds.Count -eq 0)) { break }
    if ($i -eq 0) { Write-Host "    -> killing running OpenXgram processes for update (no reboot)" }
    foreach ($p in $running) {
        Write-Host "      - $($p.Name) (PID $($p.Id))"
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
    foreach ($w in $wrapperCmds) {
        Write-Host "      - wrapper cmd.exe (PID $($w.ProcessId))"
        Stop-Process -Id $w.ProcessId -Force -ErrorAction SilentlyContinue
    }
    Start-Sleep -Milliseconds 600
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

    # rc.186 patch: Windows Defender Firewall rule 추가 — 0.0.0.0 bind 만 으로는 외부 access X.
    # 모든 OpenXgram port (transport 47300, GUI 47302 + fallback 17302/47312/27302, MCP 47301) 허용.
    try {
        $existingRule = Get-NetFirewallRule -DisplayName 'OpenXgram' -ErrorAction SilentlyContinue
        if (-not $existingRule) {
            New-NetFirewallRule -DisplayName 'OpenXgram' `
                -Direction Inbound -Action Allow -Protocol TCP `
                -LocalPort 47300,47301,47302,17302,47312,27302 `
                -Program "$INSTALL\xgram.exe" -ErrorAction SilentlyContinue | Out-Null
            Write-Host "    [OK] Firewall rule 'OpenXgram' added (allow inbound 47300-47302/17302/47312/27302)"
        }
    } catch {
        Write-Host "    [WARN] Firewall rule failed: $($_.Exception.Message)" -ForegroundColor Yellow
    }

    # daemon
    $daemonLog = Join-Path $dataDir 'daemon.log'
    $daemonProc = Start-Process -FilePath "$INSTALL\xgram.exe" `
        -ArgumentList 'daemon', '--bind', '0.0.0.0:47300', '--gui-bind', '0.0.0.0:47302' `
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
    Write-Host '==> Step 8: re-enable + restart stopped tasks/services' -ForegroundColor Cyan
    foreach ($t in $stoppedTasks) {
        Write-Host "    -> enable + start scheduled task: $t"
        # rc.186 patch: Disable 됐던 task 다시 Enable. /Run 만 으로는 disabled task fail.
        Enable-ScheduledTask -TaskName $t -ErrorAction SilentlyContinue | Out-Null
        schtasks /Run /TN $t 2>$null | Out-Null
    }
    foreach ($s in $stoppedSvcs) {
        Write-Host "    -> start service: $s"
        Start-Service -Name $s -ErrorAction SilentlyContinue
    }
}


# 8.5 (rc.203+) User Logon Scheduled Task — daemon runs in user session.
#      RATIONALE: NSSM service runs in LogonType:SERVICE session (no user token).
#                 → `wsl tmux ...` calls fail (no user session) → auto-seed (local tmux
#                 registration) + push notification (tmux inject) both broken on Zalman.
#      FIX: Run daemon in interactive user session via Scheduled Task (AtLogOn trigger).
#           User token is present → wsl.exe inherits user env → tmux session reachable.
#      Migration: stop + remove any pre-existing NSSM 'OpenXgram-Daemon' service.
$serviceName = 'OpenXgram-Daemon'
$taskName    = 'OpenXgram-Daemon-User'
$dataDir = Join-Path $env:USERPROFILE '.openxgram'
$daemonLog = Join-Path $dataDir 'daemon.log'
if (-not (Test-Path $dataDir)) { New-Item -ItemType Directory -Path $dataDir -Force | Out-Null }
Write-Host ''
Write-Host '==> Step 8.5: register User Logon Scheduled Task (daemon in user session)' -ForegroundColor Cyan

# rc.203: Whole block under EAP=Continue (Scheduled Task cmdlets occasionally emit non-fatal stderr).
$oldEAP = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
try {
    # --- 1) Graceful migration: stop + remove legacy NSSM service if present ---
    $legacySvc = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    if ($legacySvc) {
        Write-Host "    -> legacy NSSM service '$serviceName' detected — migrating to User Logon Task"
        $nssmPath = (Get-Command nssm.exe -ErrorAction SilentlyContinue).Source
        if (-not $nssmPath) {
            $candidates = @(
                "$env:LOCALAPPDATA\Microsoft\WinGet\Links\nssm.exe",
                "$env:ProgramFiles\nssm\nssm.exe",
                "${env:ProgramFiles(x86)}\nssm\nssm.exe"
            )
            foreach ($c in $candidates) { if (Test-Path $c) { $nssmPath = $c; break } }
        }
        if ($nssmPath -and (Test-Path $nssmPath)) {
            cmd /c "`"$nssmPath`" stop $serviceName" 2>&1 | Out-Null
            cmd /c "`"$nssmPath`" remove $serviceName confirm" 2>&1 | Out-Null
            Start-Sleep -Milliseconds 800
            Write-Host "    [OK] legacy NSSM service removed"
        } else {
            # Fallback: sc.exe (NSSM gone but service entry lingering)
            cmd /c "sc.exe stop $serviceName" 2>&1 | Out-Null
            cmd /c "sc.exe delete $serviceName" 2>&1 | Out-Null
            Write-Host "    [OK] legacy service deleted via sc.exe"
        }
    }

    # --- 2) Firewall rule (admin 권한 필요 — install.ps1 self-elevate) ---
    $fwExists = Get-NetFirewallRule -DisplayName 'OpenXgram' -ErrorAction SilentlyContinue
    if (-not $fwExists) {
        New-NetFirewallRule -DisplayName 'OpenXgram' -Direction Inbound -Action Allow -Protocol TCP -LocalPort 47300,47301,47302,17302,47312,27302 -Program "$INSTALL\xgram.exe" -ErrorAction SilentlyContinue | Out-Null
    }

    # --- 3) Windows Defender exclusion (Norton: no API — manual prompt below) ---
    try {
        Add-MpPreference -ExclusionPath "$INSTALL\xgram.exe" -ErrorAction SilentlyContinue
        Add-MpPreference -ExclusionPath $dataDir -ErrorAction SilentlyContinue
        Write-Host "    [OK] Defender exclusion added ($INSTALL\xgram.exe, $dataDir)"
    } catch {
        Write-Host "    [WARN] Defender exclusion skipped (non-Defender AV or insufficient privilege)" -ForegroundColor Yellow
    }

    # --- 4) Estimate Tailscale public URL (best-effort) ---
    $tailscaleIp = $null
    try {
        $tsCmd = Get-Command tailscale.exe -ErrorAction SilentlyContinue
        if ($tsCmd) {
            $tsOut = & tailscale.exe ip -4 2>$null
            if ($LASTEXITCODE -eq 0 -and $tsOut) {
                $tailscaleIp = ($tsOut | Select-Object -First 1).Trim()
            }
        }
        if (-not $tailscaleIp) {
            $tsIface = Get-NetIPAddress -AddressFamily IPv4 -InterfaceAlias '*Tailscale*' -ErrorAction SilentlyContinue
            if ($tsIface) { $tailscaleIp = ($tsIface | Select-Object -First 1).IPAddress }
        }
    } catch {}
    $transportPublicUrl = if ($tailscaleIp) { "http://${tailscaleIp}:47300" } else { $env:XGRAM_TRANSPORT_PUBLIC_URL }

    # --- 5) Build daemon args + env ---
    $daemonArgs = "daemon --data-dir `"$dataDir`" --bind 0.0.0.0:47300 --gui-bind 0.0.0.0:47302"
    $envArr = @()
    if ($env:XGRAM_KEYSTORE_PASSWORD) { $envArr += "XGRAM_KEYSTORE_PASSWORD=$env:XGRAM_KEYSTORE_PASSWORD" }
    if ($transportPublicUrl) { $envArr += "XGRAM_TRANSPORT_PUBLIC_URL=$transportPublicUrl" }

    # --- 6) Register Scheduled Task: AtLogOn, Interactive (user desktop), Highest privilege ---
    # Existing task?  Replace cleanly.
    $existing = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    if ($existing) {
        Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue | Out-Null
    }

    $action    = New-ScheduledTaskAction -Execute "$INSTALL\xgram.exe" -Argument $daemonArgs -WorkingDirectory $INSTALL
    $trigger   = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
    # RestartCount=5, RestartInterval=1m → daemon crash 후 자동 부활.
    $settings  = New-ScheduledTaskSettingsSet `
        -AllowStartIfOnBatteries `
        -DontStopIfGoingOnBatteries `
        -StartWhenAvailable `
        -RestartCount 5 `
        -RestartInterval (New-TimeSpan -Minutes 1) `
        -ExecutionTimeLimit ([TimeSpan]::Zero) `
        -MultipleInstances IgnoreNew
    # Interactive = user desktop session (token으로 WSL/tmux 접근 가능). Highest = admin elevate.
    $principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" -LogonType Interactive -RunLevel Highest

    Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Description 'OpenXgram daemon (user session, WSL/tmux reachable)' -Force | Out-Null

    # --- 7) Env vars via XML patch (Register-ScheduledTask cmdlet doesn't expose env directly) ---
    if ($envArr.Count -gt 0) {
        try {
            $task = Get-ScheduledTask -TaskName $taskName
            # New-ScheduledTaskAction has no Environment param; persist via process-wide env (user-scope).
            foreach ($pair in $envArr) {
                $k,$v = $pair -split '=',2
                [Environment]::SetEnvironmentVariable($k, $v, 'User')
            }
            Write-Host "    [OK] env vars persisted to User scope ($($envArr.Count) keys)"
        } catch {
            Write-Host "    [WARN] env var persist failed: $($_.Exception.Message)" -ForegroundColor Yellow
        }
    }

    # --- 8) Run task immediately (don't wait for next logon) ---
    Start-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
    Write-Host "    [OK] Scheduled Task '$taskName' registered + started (AtLogOn, Interactive, RunLevel=Highest, RestartCount=5)"
    if ($transportPublicUrl) { Write-Host "    -> XGRAM_TRANSPORT_PUBLIC_URL = $transportPublicUrl" }
    Write-Host "    NOTE: If using Norton/3rd-party AV, manually exclude: $INSTALL\xgram.exe" -ForegroundColor DarkGray

    # --- 9) Cleanup legacy wrapper artifacts ---
    Remove-Item (Join-Path $INSTALL 'openxgram-daemon-wrapper.cmd') -Force -ErrorAction SilentlyContinue | Out-Null
} catch {
    Write-Host "    [WARN] Scheduled Task register failed: $($_.Exception.Message)" -ForegroundColor Yellow
    Write-Host "    Manual: schtasks /create /tn $taskName /tr `"$INSTALL\xgram.exe daemon`" /sc onlogon /rl highest" -ForegroundColor Yellow
} finally {
    $ErrorActionPreference = $oldEAP
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

    # 8.7 (rc.210) WSL daemon reboot survival.
    #      Windows daemon은 Step 8.5 Task로 auto-start. WSL daemon은 install.sh 의 nohup 만
    #      걸려 있어 reboot 후 사라짐. → AtLogOn Task로 WSL 안의 xgram daemon도 자동 기동.
    #      bind: 0.0.0.0:17400 (data port), 0.0.0.0:17402 (gui port) — Windows 47300/47302 와 충돌 X.
    #      pair-host (rc.209) 가 두 daemon 자동 매칭하므로 co-exist OK.
    $wslDaemonTaskName = 'OpenXgram-WSL-Daemon-User'
    Write-Host ''
    Write-Host '==> Step 8.7: register OpenXgram-WSL-Daemon-User Scheduled Task (WSL daemon reboot survival)' -ForegroundColor Cyan
    try {
        # WSL user 자동 detect (default distro).
        $wslUser = ''
        try { $wslUser = (wsl.exe -- whoami 2>$null | Out-String).Trim() } catch {}
        if (-not $wslUser) {
            Write-Host '    [WARN] WSL whoami failed — Task 등록 skip. WSL distro 가 init 안 됐을 수 있음.' -ForegroundColor Yellow
        } else {
            # WSL 안에 xgram 바이너리 있는지 확인 (없으면 안내 후 skip).
            $xgramExists = (wsl.exe -- bash -lc 'test -x "$HOME/.local/bin/xgram" && echo OK || echo NO' 2>$null | Out-String).Trim()
            if ($xgramExists -ne 'OK') {
                Write-Host '    [SKIP] WSL 안에 ~/.local/bin/xgram 없음.' -ForegroundColor Yellow
                Write-Host '           WSL 에서 먼저 install:  curl -sL https://openxgram.org/install.sh | bash' -ForegroundColor Yellow
            } else {
                # Existing task?  Replace cleanly.
                $existingWslD = Get-ScheduledTask -TaskName $wslDaemonTaskName -ErrorAction SilentlyContinue
                if ($existingWslD) {
                    Unregister-ScheduledTask -TaskName $wslDaemonTaskName -Confirm:$false -ErrorAction SilentlyContinue | Out-Null
                }

                # bash -lc payload: source daemon.env (if exists) → nohup xgram daemon (bg).
                # 17400 = data port (Windows 47300 과 분리), 17402 = gui port (Windows 47302 와 분리).
                $wslDCmd = 'if [ -f ~/.openxgram/daemon.env ]; then . ~/.openxgram/daemon.env; fi; mkdir -p ~/.openxgram; nohup ~/.local/bin/xgram daemon --bind 0.0.0.0:17400 --gui-bind 0.0.0.0:17402 > ~/.openxgram/wsl-daemon.log 2>&1 &'
                $wslDArg = "-- bash -lc `"$wslDCmd`""

                # rc.211 — schtasks.exe 로 직접 등록 (New-ScheduledTaskPrincipal -UserId 의 SID mapping
                # 실패 회피). Step 8.5 의 PowerShell cmdlet 방식은 일부 환경 (Microsoft Account,
                # AzureAD-joined, 비-domain) 에서 "No mapping between account names and security IDs"
                # error 발생. schtasks /RU 는 current user 를 native 로 해석 → 항상 작동.
                $wslDTr = "wsl.exe -- bash -lc `"$wslDCmd`""
                # schtasks XML escape: " → \"  (cmd.exe 의 quote 규칙).
                $wslDTrEsc = $wslDTr -replace '"', '\"'
                $schtasksArgs = @(
                    '/Create',
                    '/TN', $wslDaemonTaskName,
                    '/TR', "`"$wslDTrEsc`"",
                    '/SC', 'ONLOGON',
                    '/RU', $env:USERNAME,
                    '/RL', 'HIGHEST',
                    '/F'
                )
                $schtasksOut = & schtasks.exe @schtasksArgs 2>&1
                if ($LASTEXITCODE -ne 0) {
                    Write-Host "    [WARN] schtasks /Create failed (exit=$LASTEXITCODE): $schtasksOut" -ForegroundColor Yellow
                    Write-Host '    Fallback: New-ScheduledTask*' -ForegroundColor DarkGray
                    # Fallback: PowerShell cmdlet path (may fail on MSA accounts).
                    $wslDAction    = New-ScheduledTaskAction -Execute 'wsl.exe' -Argument $wslDArg
                    $wslDTrigger   = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
                    $wslDSettings  = New-ScheduledTaskSettingsSet `
                        -AllowStartIfOnBatteries `
                        -DontStopIfGoingOnBatteries `
                        -StartWhenAvailable `
                        -RestartCount 5 `
                        -RestartInterval (New-TimeSpan -Minutes 1) `
                        -ExecutionTimeLimit ([TimeSpan]::Zero) `
                        -MultipleInstances IgnoreNew
                    # LogonType=Interactive + -UserId 생략 → current logged-on user 자동.
                    $wslDPrincipal = New-ScheduledTaskPrincipal -GroupId 'S-1-5-32-545' -RunLevel Highest
                    Register-ScheduledTask -TaskName $wslDaemonTaskName -Action $wslDAction -Trigger $wslDTrigger -Settings $wslDSettings -Principal $wslDPrincipal -Description "OpenXgram WSL daemon (user: $wslUser, ports 17400/17402, reboot survival)" -Force | Out-Null
                }

                # Trigger immediately so we don't wait for next logon.
                Start-ScheduledTask -TaskName $wslDaemonTaskName -ErrorAction SilentlyContinue
                Start-Sleep -Seconds 2
                Write-Host "    [OK] Scheduled Task '$wslDaemonTaskName' registered + started (user=$wslUser, bind 0.0.0.0:17400 / 0.0.0.0:17402)"
                Write-Host '    -> pair-host 가 Windows 47300 daemon 과 자동 매칭.' -ForegroundColor DarkGray
            }
        }
    } catch {
        Write-Host "    [WARN] WSL daemon Scheduled Task register failed: $($_.Exception.Message)" -ForegroundColor Yellow
        Write-Host "    Manual: schtasks /create /tn $wslDaemonTaskName /tr `"wsl.exe -- bash -lc 'nohup ~/.local/bin/xgram daemon --bind 0.0.0.0:17400 --gui-bind 0.0.0.0:17402 > ~/.openxgram/wsl-daemon.log 2>&1 &'`" /sc onlogon /rl highest" -ForegroundColor Yellow
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
