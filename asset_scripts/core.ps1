# Hok minimal helper — core functions for package script compatibility.
# Embedded in hok binary via include_str!.

function Get-HelperPath($name) {
    $shims = Join-Path $env:SCOOP "shims"
    $exe = Join-Path $shims "$name.exe"
    if (Test-Path $exe) { return $exe }
    $found = Get-Command "$name.exe" -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }

    # Auto-install via hok (one-time dependency provisioning)
    $pkgMap = @{
        '7z' = '7zip'; '7zip' = '7zip'
        'innounp' = 'innounp'
        'aria2c' = 'aria2'
        'gsudo' = 'gsudo'
        'git' = 'git'
    }
    $pkg = if ($pkgMap.ContainsKey($name)) { $pkgMap[$name] } else { $name }
    info "Installing dependency: $pkg"
    & hok install $pkg --assume-yes

    # Retry after install
    if (Test-Path $exe) { return $exe }
    $found = Get-Command "$name.exe" -ErrorAction SilentlyContinue
    if ($found) { return $found.Source }
    return $name  # fallback — will probably fail at usage
}

function friendly_path($path) {
    $scoop = $env:SCOOP
    if ($path -and $scoop -and $path.StartsWith($scoop, [StringComparison]::OrdinalIgnoreCase)) {
        return "~\..$($path.Substring($scoop.Length))"
    }
    return $path
}

function ensure($path) {
    if (-not (Test-Path $path)) {
        $null = New-Item -ItemType Directory -Path $path -Force
    }
    return $path
}

function is_admin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal] $identity
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function New-DirectoryJunction($Path, $Target) {
    if (Test-Path $Path) { return }
    $null = New-Item -ItemType Junction -Path $Path -Target $Target -Force
}

function info($msg) {
    Write-Host "$msg" -ForegroundColor Green
}

function warn($msg) {
    Write-Host "$msg" -ForegroundColor Yellow
}

function error($msg) {
    Write-Host "$msg" -ForegroundColor Red
}

function abort($msg) {
    Write-Host "$msg" -ForegroundColor Red
    throw $msg
}

function Invoke-ExternalCommand($FilePath, $ArgumentList, $RunAs, $Quiet) {
    if ($RunAs) {
        Start-Process -FilePath $FilePath -ArgumentList $ArgumentList -Verb RunAs -Wait
        return
    }
    $p = Start-Process -FilePath $FilePath -ArgumentList $ArgumentList -NoNewWindow -Wait -PassThru
    if ($Quiet) { return }
    return $p.ExitCode
}

# Registry helpers (Scoop-compatible)
function WriteReg($Path, $Name, $Value, $Type) {
    if (-not (Test-Path $Path)) { $null = New-Item -Path $Path -Force }
    if ($Type) { Set-ItemProperty -Path $Path -Name $Name -Value $Value -Type $Type }
    else { Set-ItemProperty -Path $Path -Name $Name -Value $Value }
}

function Get-RegKey($Path, $Name) {
    if (Test-Path $Path) { return (Get-ItemProperty -Path $Path -Name $Name -ErrorAction SilentlyContinue).$Name }
    return $null
}

function New-RegKey($Path) {
    if (-not (Test-Path $Path)) { $null = New-Item -Path $Path -Force }
}

function Remove-RegKey($Path) {
    if (Test-Path $Path) { Remove-Item -Path $Path -Recurse -Force -ErrorAction SilentlyContinue }
}

# Environment variable helpers
function Write-Env($Name, $Value, $Global) {
    $scope = if ($Global) { 'Machine' } else { 'User' }
    [Environment]::SetEnvironmentVariable($Name, $Value, $scope)
}

function Remove-Env($Name, $Global) {
    $scope = if ($Global) { 'Machine' } else { 'User' }
    [Environment]::SetEnvironmentVariable($Name, $null, $scope)
}

# Version helpers (minimal — used by checkver/update flows)
function Get-InstalledVersion($App) {
    $verDir = Join-Path $env:SCOOP "apps" $App
    if (Test-Path $verDir) {
        return (Get-ChildItem $verDir -Directory | Where-Object { $_.Name -ne 'current' } | Sort-Object Name -Descending | Select-Object -First 1).Name
    }
    return $null
}

function Select-CurrentVersion($AppName, $Global) {
    $verDir = Join-Path $env:SCOOP "apps" $AppName
    if (Test-Path "$verDir\current") {
        $link = (Get-Item "$verDir\current" -ErrorAction SilentlyContinue).Target
        if ($link) { return (Split-Path $link -Leaf) }
    }
    return $null
}

function Get-Version($App) {
    return Get-InstalledVersion $App
}
