# Wren Terminal - Windows Terminal profile 集成脚本
# 由 MSI 在 Install / Uninstall 时自动调用，也可手动运行。
# 以 SYSTEM 权限运行时会遍历 C:\Users 下所有用户，把 Wren 注入每个用户的
# Windows Terminal settings.json（正式版 + 预览版）。

param(
    [ValidateSet("Install", "Uninstall")]
    [string]$Mode = "Install"
)

$ErrorActionPreference = "SilentlyContinue"

# 在脚本顶层构造 UTF-8 无 BOM 编码（避免在某些执行上下文中 ::new() 解析异常）
$Utf8NoBom = New-Object System.Text.UTF8Encoding -ArgumentList @($false)
if ($null -eq $Utf8NoBom) {
    $Utf8NoBom = [System.Text.Encoding]::UTF8
}

$ProfileGuid  = "{a3f1c7e9-5b2d-4f8a-9c1e-7d6b0a4f2e90}"
$ProfileName  = "Wren Terminal"

$WrenExe = Join-Path $PSScriptRoot "wren.exe"
$WrenIco = Join-Path $PSScriptRoot "wren.ico"

$LogPath = Join-Path $env:TEMP "wren-terminal-profile.log"
function Write-Log([string]$Message) {
    try {
        Add-Content -Path $LogPath -Value ("{0}  {1}" -f (Get-Date -Format "yyyy-MM-dd HH:mm:ss"), $Message) -Encoding UTF8
    } catch { }
}

Write-Log "=== $Mode start (PSRoot=$PSScriptRoot) ==="

# 测试钩子：设置 WREN_USERS_ROOT 可指向临时目录（不影响真实用户配置）
$usersRoot = $env:WREN_USERS_ROOT
if (-not $usersRoot) { $usersRoot = "C:\Users" }

# 收集所有 Windows Terminal 的 settings.json
$settingsFiles = @()
Get-ChildItem -Path $usersRoot -Directory -Force -ErrorAction SilentlyContinue | ForEach-Object {
    $userPackages = Join-Path $_.FullName "AppData\Local\Packages"
    if (-not (Test-Path $userPackages)) { return }
    Get-ChildItem -Path $userPackages -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "Microsoft.WindowsTerminal*" } |
        ForEach-Object {
            $sf = Join-Path $_.FullName "LocalState\settings.json"
            if (Test-Path $sf) { $settingsFiles += $sf }
        }
}

Write-Log ("Found {0} settings file(s)" -f $settingsFiles.Count)

foreach ($sf in $settingsFiles) {
    try {
        $raw = Get-Content -Path $sf -Raw -Encoding UTF8
        if ([string]::IsNullOrWhiteSpace($raw)) { Write-Log "SKIP empty: $sf"; continue }

        $settings = $raw | ConvertFrom-Json
        if (-not $settings.profiles -or -not $settings.profiles.list) {
            Write-Log "SKIP no profiles.list: $sf"
            continue
        }

        $list = @($settings.profiles.list)
        $matched = @($list | Where-Object {
            $_.guid -eq $ProfileGuid -or $_.name -eq $ProfileName
        })

        $changed = $false

        if ($Mode -eq "Install") {
            if ($matched.Count -gt 0) {
                Write-Log "ALREADY present: $sf"
                continue
            }
            $profile = [ordered]@{
                guid              = $ProfileGuid
                name              = $ProfileName
                commandline       = '"' + $WrenExe + '"'
                icon              = $WrenIco
                startingDirectory = "%USERPROFILE%"
                colorScheme       = "One Half Dark"
                fontFace          = "Cascadia Code"
                fontSize          = 11
                hidden            = $false
            }
            $settings.profiles.list = [object[]]@($list) + [object[]]@($profile)
            $changed = $true
            Write-Log "ADD profile: $sf"
        }
        else {
            if ($matched.Count -eq 0) {
                Write-Log "NOT present, nothing to remove: $sf"
                continue
            }
            $remaining = @($list | Where-Object {
                $_.guid -ne $ProfileGuid -and $_.name -ne $ProfileName
            })
            $settings.profiles.list = [object[]]$remaining
            $changed = $true
            Write-Log "REMOVE profile: $sf"
        }

        if ($changed) {
            $out = $settings | ConvertTo-Json -Depth 100
            [System.IO.File]::WriteAllText($sf, $out, $Utf8NoBom)
            Write-Log "WROTE: $sf"
        }
    }
    catch {
        Write-Log ("ERROR on {0}: {1}" -f $sf, $_.Exception.Message)
    }
}

Write-Log "=== $Mode done ==="
