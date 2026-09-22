#Requires -Version 5.1
<#
.SYNOPSIS
    Build hok in one of its two SQLite shapes.

.DESCRIPTION
    `hok`'s manifest cache uses SQLite, and there are two ways to get it:

    * `bundled` (default) — compile the SQLite amalgamation into the binary.
      No external dependency, works everywhere, larger binary.
    * `winsqlite3` — link the SQLite that ships with Windows
      (`C:\Windows\System32\winsqlite3.dll`). About 1 MB smaller, but it makes
      the binary depend on a Windows component and on the Windows SDK at build
      time.

    The shape is chosen by environment variables read by `libsqlite3-sys`, so
    this script is the supported way to get the non-default one: it stages the
    SDK's import library under the name `libsqlite3-sys` asks for and sets the
    three variables **for its own process only**. Nothing persistent is
    written — no user or machine environment variable, no
    `.cargo/config.toml` — so a plain `cargo build` keeps producing the
    bundled shape no matter how often this script has run.

    `.winsqlite3/` (the staging directory, gitignored) is the only thing left
    behind, and `-Clean` removes it.

    `winsqlite3` is x64/arm64 only: on x86 the import library exports
    __stdcall-decorated names (`_sqlite3_column_name@8`) that rusqlite's cdecl
    `extern "C"` declarations cannot resolve, so i686 keeps the bundled build.
    `crates/libscoop/build.rs` fails the build if both are asked for at once.

.EXAMPLE
    Default shape (same as `cargo build --release -p hok`):

        pwsh scripts/build-hok.ps1

.EXAMPLE
    The winsqlite3 shape, with the resulting dependencies and size checked:

        pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Verify

.EXAMPLE
    Debug winsqlite3 build, and drop the staging directory first:

        pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Configuration debug -Clean

.NOTES
    `-Package libscoop` builds the rlib only. That cannot be used to check
    whether the winsqlite3 link works: an rlib is not linked, so a missing
    `sqlite3.lib` stays invisible until something links a binary or a test.
    Use `dumpbin /dependents` on `hok.exe`, which is what -Verify does.
#>
[CmdletBinding()]
param(
    # Which SQLite shape to build. `bundled` is the default on purpose: this
    # script never changes the default shape implicitly.
    [ValidateSet('bundled', 'winsqlite3')]
    [string]$Mode = 'bundled',

    [ValidateSet('release', 'debug')]
    [string]$Configuration = 'release',

    # rlib only; see the note about -Verify above.
    [ValidateSet('hok', 'libscoop')]
    [string]$Package = 'hok',

    # Rust target triple. Defaults to the host, and decides which winsqlite3
    # import library is staged and which dumpbin is used.
    [string]$Target,

    # Remove the staging directory (only that; never `cargo clean`, which would
    # throw away other people's artifacts in a shared target directory).
    [switch]$Clean,

    # After building, print `dumpbin /dependents` and the binary size, and
    # assert that the shape is the one that was asked for.
    [switch]$Verify,

    # Windows 10/11 SDK root, e.g. `C:\Program Files (x86)\Windows Kits\10`.
    # Auto-detected (vswhere, then the SDK registration, then the default
    # install path) when omitted.
    [string]$SdkRoot
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Sizes of target/release/hok.exe as measured in this repository (Windows SDK
# 10.0.26100, Rust 1.98, x86_64-pc-windows-msvc). Only used to put the
# -Verify output in context; a drift here is not an error.
$ReferenceSizeBytes = @{
    bundled    = 8124928
    winsqlite3 = 7088640
}

$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$stageDir = Join-Path $repoRoot '.winsqlite3'

function Get-VsWherePath {
    if (-not ${env:ProgramFiles(x86)}) { return $null }
    $path = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path -LiteralPath $path) { return $path }
    return $null
}

function Get-CandidateSdkRoots {
    # An explicit root is authoritative: it is how a caller points at an SDK
    # the auto-detection below would not consider.
    if ($SdkRoot) { return @($SdkRoot) }

    $roots = [System.Collections.Generic.List[string]]::new()

    if ($env:WindowsSdkDir) { $roots.Add($env:WindowsSdkDir) }

    $vswhere = Get-VsWherePath
    if ($vswhere) {
        $install = (& $vswhere -latest -products * -property installationPath 2>$null | Select-Object -First 1)
        if ($install) {
            # <install>\..\..\.. == %ProgramFiles(x86)%; the SDK sits beside the
            # Visual Studio installation there.
            $base = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $install.Trim()))
            if ($base) { $roots.Add((Join-Path $base 'Windows Kits\10')) }
        }
    }

    # What the SDK installer itself registers; the canonical location.
    foreach ($key in @(
            'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots',
            'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots'
        )) {
        if (-not (Test-Path -LiteralPath $key)) { continue }
        $value = (Get-ItemProperty -LiteralPath $key -Name 'KitsRoot10' -ErrorAction SilentlyContinue).KitsRoot10
        if ($value) { $roots.Add($value) }
    }

    foreach ($programFiles in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if ($programFiles) { $roots.Add((Join-Path $programFiles 'Windows Kits\10')) }
    }

    return $roots
}

function Get-SdkVersionsDescending {
    param([string]$Root)

    $libDir = Join-Path $Root 'Lib'
    if (-not (Test-Path -LiteralPath $libDir)) { return @() }

    # Newest installed SDK first. Versions are `10.0.<build>.0`, so they only
    # sort correctly as versions, not as strings.
    $versions = foreach ($dir in Get-ChildItem -LiteralPath $libDir -Directory -ErrorAction SilentlyContinue) {
        $parsed = $null
        if ([version]::TryParse($dir.Name, [ref]$parsed)) { $parsed }
    }

    return @($versions | Sort-Object -Descending)
}

function Resolve-Winsqlite3Sdk {
    param([string]$Architecture)

    $searched = [System.Collections.Generic.List[string]]::new()
    $tried = Get-CandidateSdkRoots

    foreach ($candidate in $tried) {
        if (-not $candidate) { continue }
        $root = $candidate.TrimEnd('\')
        if (-not (Test-Path -LiteralPath $root)) {
            $searched.Add("$root (missing)")
            continue
        }

        $versions = @(Get-SdkVersionsDescending -Root $root)
        if (-not $versions.Count) { $searched.Add("$root (no Lib\<version> directories)") }

        foreach ($version in $versions) {
            $header = Join-Path $root "Include\$version\um\winsqlite\winsqlite3.h"
            $lib = Join-Path $root "Lib\$version\um\$Architecture\winsqlite3.lib"
            $searched.Add("$root ($version)")

            if ((Test-Path -LiteralPath $header) -and (Test-Path -LiteralPath $lib)) {
                return [pscustomobject]@{
                    Root    = $root
                    Version = $version
                    Header  = $header
                    Lib     = $lib
                }
            }
        }
    }

    # Written out rather than thrown: PowerShell's error formatter collapses the
    # newlines of an exception message into spaces, which turns this into one
    # unreadable line.
    Write-Host (@(
            "Could not find winsqlite3 for $Architecture in any Windows SDK.",
            '',
            'Searched:',
            ((@($searched | Select-Object -Unique) | ForEach-Object { "  $_" }) -join [Environment]::NewLine),
            '',
            'winsqlite3.lib and winsqlite3.h ship with the Windows 10 SDK (10.0.16299 and',
            'later) and with Windows 11 SDKs. Install the "Windows SDK" component in the',
            'Visual Studio Installer, or point at an existing one with:',
            '',
            "    pwsh scripts/build-hok.ps1 -Mode winsqlite3 -SdkRoot 'C:\Program Files (x86)\Windows Kits\10'",
            '',
            'Or build the default shape, which needs no SDK and no staging:',
            '',
            '    pwsh scripts/build-hok.ps1'
        ) -join [Environment]::NewLine) -ForegroundColor Red
    exit 1
}

function Get-DumpbinPath {
    param([string]$Architecture)

    $hostDir = switch ($Architecture) {
        'x64' { 'Hostx64\x64' }
        'x86' { 'Hostx64\x86' }
        'arm64' { 'Hostx64\arm64' }
        default { 'Hostx64\x64' }
    }

    $vswhere = Get-VsWherePath
    if (-not $vswhere) { return $null }

    $install = (& $vswhere -latest -products * -property installationPath 2>$null | Select-Object -First 1)
    if (-not $install) { return $null }

    $toolsDir = Join-Path $install.Trim() 'VC\Tools\MSVC'
    if (-not (Test-Path -LiteralPath $toolsDir)) { return $null }

    foreach ($version in (Get-ChildItem -LiteralPath $toolsDir -Directory | Sort-Object Name -Descending)) {
        $candidate = Join-Path $version.FullName "bin\$hostDir\dumpbin.exe"
        if (Test-Path -LiteralPath $candidate) { return $candidate }
    }

    return $null
}

function Get-HostTriple {
    $line = & rustc -vV 2>$null | Where-Object { $_ -like 'host:*' } | Select-Object -First 1
    if (-not $line) { return $null }
    return ($line -replace '^host:\s*', '').Trim()
}

function Get-TargetArchitecture {
    $triple = if ($Target) { $Target } elseif ($env:CARGO_BUILD_TARGET) { $env:CARGO_BUILD_TARGET } else { Get-HostTriple }
    if (-not $triple) { return $null }

    $arch = ($triple -split '-')[0]
    switch ($arch) {
        'x86_64' { return 'x64' }
        'i686' { return 'x86' }
        'i586' { return 'x86' }
        'aarch64' { return 'arm64' }
        default { return $arch }
    }
}

# --- staging directory -------------------------------------------------------

if ($Clean) {
    if (Test-Path -LiteralPath $stageDir) {
        Remove-Item -LiteralPath $stageDir -Recurse -Force
        Write-Host "Removed staging directory $stageDir"
    } else {
        Write-Host "Staging directory $stageDir does not exist (nothing to clean)"
    }
}

$architecture = Get-TargetArchitecture

# Assigned in statements rather than as `if` expressions: an empty array coming
# out of an `if` is unrolled to nothing, so `@()` would not survive as an array.
$profileArgs = @()
if ($Configuration -eq 'release') { $profileArgs = @('--release') }
$targetArgs = @()
if ($Target) { $targetArgs = @('--target', $Target) }

# --- winsqlite3 shape --------------------------------------------------------

if ($Mode -eq 'winsqlite3') {
    if ($architecture -notin @('x64', 'arm64')) {
        Write-Host (@(
                "-Mode winsqlite3 cannot be used for '$architecture' targets.",
                '',
                'winsqlite3.dll exports its API with __stdcall on x86 (winsqlite3.h defines',
                'SQLITE_APICALL as __stdcall when NTDDI_VERSION >= NTDDI_WIN10_RS1), so',
                'winsqlite3.lib carries decorated names such as _sqlite3_column_name@8.',
                'rusqlite and libsqlite3-sys declare those functions as cdecl `extern "C"`,',
                'so the link cannot resolve them and fails with:',
                '',
                '    LNK2019: unresolved external symbol _sqlite3_column_name',
                '',
                'Build the default bundled shape for that target instead:',
                '',
                "    pwsh scripts/build-hok.ps1$(if ($Target) { " -Target $Target" })"
            ) -join [Environment]::NewLine) -ForegroundColor Red
        exit 1
    }

    $sdk = Resolve-Winsqlite3Sdk -Architecture $architecture

    New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
    $stageHeader = Join-Path $stageDir 'sqlite3.h'
    $stageLib = Join-Path $stageDir 'sqlite3.lib'
    Copy-Item -LiteralPath $sdk.Header -Destination $stageHeader -Force
    Copy-Item -LiteralPath $sdk.Lib -Destination $stageLib -Force

    # Process-scoped only. Setting these through setx, the user environment or
    # .cargo/config.toml would silently turn every later build in the repository
    # into the winsqlite3 shape — and break it wherever staging is missing.
    $env:LIBSQLITE3_SYS_USE_PKG_CONFIG = '1'
    $env:SQLITE3_LIB_DIR = $stageDir
    $env:SQLITE3_INCLUDE_DIR = $stageDir

    Write-Host "Mode winsqlite3: staged SDK $($sdk.Version) ($architecture)"
    Write-Host "  winsqlite3.lib -> $stageLib"
    Write-Host "  winsqlite3.h   -> $stageHeader"
    Write-Host ''
}

# --- effective configuration, always printed ---------------------------------

Write-Host 'Effective build:'
Write-Host "  shape          : $Mode"
Write-Host "  configuration  : $Configuration"
Write-Host "  package        : $Package"
Write-Host "  target         : $(if ($Target) { $Target } else { 'host default' })"
Write-Host ''
Write-Host 'Environment used by this build (process scope; nothing persistent was written):'
if ($Mode -eq 'winsqlite3') {
    Write-Host "  `$env:LIBSQLITE3_SYS_USE_PKG_CONFIG = '1'"
    Write-Host "  `$env:SQLITE3_LIB_DIR     = '$stageDir'"
    Write-Host "  `$env:SQLITE3_INCLUDE_DIR = '$stageDir'"
} else {
    # -Mode bundled must mean bundled, but these are read straight by
    # libsqlite3-sys, so a value left over in the shell would override the
    # choice. Report it instead of quietly building the other shape.
    $stray = @(
        'LIBSQLITE3_SYS_USE_PKG_CONFIG'
        'SQLITE3_LIB_DIR'
        'SQLITE3_INCLUDE_DIR'
    ) | Where-Object { Test-Path "Env:$_" }

    if ($stray) {
        Write-Host "  already set in this shell: $($stray -join ', ')" -ForegroundColor Yellow
        Write-Host '  Those select the winsqlite3 shape, so this build is NOT the bundled one.' -ForegroundColor Yellow
        Write-Host '  Clear them first:' -ForegroundColor Yellow
        Write-Host "    Remove-Item $(($stray | ForEach-Object { "Env:$_" }) -join ', ')" -ForegroundColor Yellow
    } else {
        Write-Host '  (none — bundled SQLite is compiled in)'
    }
}
Write-Host ''
Write-Host 'Equivalent cargo commands:'
Write-Host "  cargo build -p hok-shim$(if ($profileArgs) { ' ' + ($profileArgs -join ' ') })"
Write-Host "  cargo build -p $Package$(if ($profileArgs) { ' ' + ($profileArgs -join ' ') })$(if ($targetArgs) { ' ' + ($targetArgs -join ' ') })"
Write-Host ''

# --- build -------------------------------------------------------------------

Push-Location $repoRoot
try {
    # libscoop's build script embeds the shim binaries, so they are needed even
    # for `-Package libscoop`.
    & cargo build -p hok-shim @profileArgs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    & cargo build -p $Package @profileArgs @targetArgs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}

$targetRoot = if ($Target) { Join-Path $repoRoot "target\$Target" } else { Join-Path $repoRoot 'target' }
$exePath = Join-Path $targetRoot "$Configuration\hok.exe"

if ($Package -ne 'hok') { $exePath = $null }

# --- verification ------------------------------------------------------------

if ($Verify) {
    if (-not $exePath) {
        Write-Host (@(
                '-Verify needs a linked binary, and -Package libscoop only produces an rlib.',
                'An rlib is not linked, so a missing sqlite3.lib stays invisible here — build',
                '-Package hok (the default) to check the shape.'
            ) -join [Environment]::NewLine) -ForegroundColor Red
        exit 1
    }

    if (-not (Test-Path -LiteralPath $exePath)) {
        Write-Host "Expected binary not found: $exePath" -ForegroundColor Red
        exit 1
    }

    $dumpbin = Get-DumpbinPath -Architecture $architecture
    if (-not $dumpbin) {
        Write-Host (@(
                'dumpbin.exe was not found (Visual Studio with the C++ tools is required).',
                'Run "dumpbin /dependents <hok.exe>" from a Developer Command Prompt to',
                'check the shape by hand.'
            ) -join [Environment]::NewLine) -ForegroundColor Red
        exit 1
    }

    $dependents = & $dumpbin /nologo /dependents $exePath
    $dlls = $dependents | Where-Object { $_ -match '^\s+\S+\.dll\s*$' } | ForEach-Object { $_.Trim().ToLowerInvariant() }
    $usesWinsqlite3 = $dlls -contains 'winsqlite3.dll'
    $usesSqlite3 = $dlls -contains 'sqlite3.dll'

    $size = (Get-Item -LiteralPath $exePath).Length
    $reference = $ReferenceSizeBytes[$Mode]
    $other = if ($Mode -eq 'bundled') { 'winsqlite3' } else { 'bundled' }

    Write-Host ''
    Write-Host 'Verification:'
    Write-Host "  binary          : $exePath"
    Write-Host "  size            : $size bytes ($([math]::Round($size / 1MB, 2)) MiB)"
    Write-Host "  reference $($Mode.PadRight(10)): $reference bytes ($([math]::Round($reference / 1MB, 2)) MiB), delta $($size - $reference)"
    Write-Host "  reference $($other.PadRight(10)): $($ReferenceSizeBytes[$other]) bytes ($([math]::Round($ReferenceSizeBytes[$other] / 1MB, 2)) MiB)"
    Write-Host "  winsqlite3.dll  : $(if ($usesWinsqlite3) { 'imported' } else { 'not imported' })"
    Write-Host "  sqlite3.dll     : $(if ($usesSqlite3) { 'imported (unexpected)' } else { 'not imported' })"
    Write-Host ''
    Write-Host '  sqlite-related dependencies from dumpbin /dependents:'
    foreach ($dll in ($dlls | Where-Object { $_ -like '*sqlite*' })) { Write-Host "    $dll" }
    if (-not ($dlls | Where-Object { $_ -like '*sqlite*' })) { Write-Host '    (none)' }
    Write-Host ''

    $ok = if ($Mode -eq 'winsqlite3') { $usesWinsqlite3 -and -not $usesSqlite3 } else { -not $usesWinsqlite3 }
    if ($ok) {
        Write-Host "OK: the $Mode shape was produced." -ForegroundColor Green
    } else {
        Write-Host "FAILED: expected the $Mode shape, but winsqlite3.dll is $(if ($usesWinsqlite3) { 'imported' } else { 'not imported' })." -ForegroundColor Red
        exit 1
    }
}
