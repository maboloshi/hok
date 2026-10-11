<#
.SYNOPSIS
    Guardrail benchmark: hok vs official Scoop on the same batch of commands.

.DESCRIPTION
    Phase-5 guardrail: verifies hok is behaviorally equivalent to official Scoop
    and not slower on a shared batch of commands. Each command runs on both tools;
    the script captures exit code, output and wall-clock time, then reports
    equivalence (exit code + normalized text) and a timing ratio.

    Each invocation uses Start-Process with stdout/stderr redirected to temp files
    and a per-case timeout (20s), so a single hung command is reported as TIMEOUT
    instead of blocking the whole run.

    Safe/read-only commands run by default. Network/stateful commands (status,
    checkver, download, install) only run with -IncludeNetwork, and are skipped
    automatically when the network probe fails so the script never errors out on
    an unavailable environment.

.PARAMETER HokPath
    Path to the hok executable to benchmark. Defaults to the workspace debug build
    (target/debug/hok.exe).

.PARAMETER ScoopCmd
    Command used to invoke official Scoop. Defaults to 'scoop' (resolved from PATH;
    a .ps1 shim is run through powershell.exe -File, so its timing includes PS startup).

.PARAMETER IncludeNetwork
    Also run network/stateful commands (status/checkver/download/install).

.PARAMETER OutputPath
    Directory for the per-case captured outputs and the summary report.
    Defaults to scripts/benchmark-output/.

.EXAMPLE
    ./scripts/benchmark_vs_scoop.ps1

    ./scripts/benchmark_vs_scoop.ps1 -HokPath .\target\release\hok.exe -IncludeNetwork
#>
[CmdletBinding()]
param(
    [string]$HokPath = (Join-Path $PSScriptRoot '..\target\debug\hok.exe'),
    [string]$ScoopCmd = 'scoop',
    [switch]$IncludeNetwork,
    [string]$OutputPath = (Join-Path $PSScriptRoot 'benchmark-output')
)

$ErrorActionPreference = 'Continue'

# Ensure the report directory exists.
New-Item -ItemType Directory -Force -Path $OutputPath | Out-Null

function Invoke-Tool {
    param([string]$Cmd, [string[]]$CmdArgs, [string]$Work)
    $outFile = Join-Path $OutputPath "$Work.stdout.tmp"
    $errFile = Join-Path $OutputPath "$Work.stderr.tmp"
    Remove-Item $outFile, $errFile -ErrorAction SilentlyContinue
    $sp = @{
        FilePath                  = $Cmd
        ArgumentList              = $CmdArgs
        NoNewWindow               = $true
        PassThru                  = $true
        RedirectStandardOutput    = $outFile
        RedirectStandardError     = $errFile
    }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process @sp
    $done = $p.WaitForExit(20000)
    $sw.Stop()
    if (-not $done) { $p.Kill() }
    $stdout = if (Test-Path $outFile) { Get-Content $outFile -Raw } else { '' }
    return [pscustomobject]@{
        ExitCode = if ($done) { $p.ExitCode } else { -1 }
        Stdout   = $stdout
        Ms       = $sw.Elapsed.TotalMilliseconds
        TimedOut = -not $done
    }
}

function Normalize([string]$Text) {
    # Trim trailing whitespace per line, drop blank lines, sort lines so
    # unordered list/table output is compared regardless of column width.
    $lines = $Text -split "`r?`n" | ForEach-Object { $_.TrimEnd() } | Where-Object { $_.Trim().Length -gt 0 }
    return ($lines | Sort-Object) -join "`n"
}

$networkCases = @(
    @{ name = 'status';   args = @('status') },
    @{ name = 'checkver'; args = @('checkver', 'jq') },
    @{ name = 'download'; args = @('download', 'jq') },
    @{ name = 'install';  args = @('install', 'jq') }
)
$localCases = @(
    @{ name = 'list';   args = @('list') },
    @{ name = 'search'; args = @('search', 'jq') },
    @{ name = 'info';   args = @('info', 'jq') },
    @{ name = 'which';  args = @('which', 'git') },
    @{ name = 'prefix'; args = @('prefix', 'git') }
)

# Resolve the hok executable; skip everything if missing.
$hokFull = if ([System.IO.Path]::IsPathRooted($HokPath)) { $HokPath } else { (Resolve-Path $HokPath -ErrorAction SilentlyContinue).Path }
if (-not $hokFull -or -not (Test-Path $hokFull)) {
    Write-Warning "hok executable not found at '$HokPath' — build it first (cargo build) or pass -HokPath."
}

# Resolve official Scoop; if unavailable, skip the Scoop side gracefully.
$scoopFull = (Get-Command $ScoopCmd -ErrorAction SilentlyContinue).Source
$scoopInvoke = $null
if ($scoopFull) {
    if ($scoopFull -like '*.ps1') {
        # .ps1 shim cannot be started directly; route through powershell.exe -File.
        $scoopInvoke = @{ Cmd = 'powershell.exe'; Args = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $scoopFull) }
    } else {
        $scoopInvoke = @{ Cmd = $scoopFull; Args = @() }
    }
} else {
    Write-Warning "scoop command '$ScoopCmd' not found — skipping Scoop comparisons."
}

# Probe network reachability once (used to gate network/stateful cases).
$netOk = $false
if ($IncludeNetwork) {
    try {
        $null = Invoke-WebRequest -Uri 'https://github.com' -Method Head -TimeoutSec 8 -UseBasicParsing
        $netOk = $true
    } catch {
        Write-Warning "Network probe failed — skipping network/stateful cases. ($($_.Exception.Message))"
    }
}

$cases = @()
if ($hokFull) { $cases += $localCases }
if ($hokFull -and $IncludeNetwork) { $cases += $networkCases }

$report = [System.Collections.Generic.List[object]]::new()

foreach ($c in $cases) {
    $name = $c.name
    $isNet = $c.args[0] -in @('status', 'checkver', 'download', 'install')
    if ($isNet -and -not $netOk) {
        $report.Add([pscustomobject]@{ Case = $name; Equiv = 'SKIP'; Hok = $null; Scoop = $null; HokMs = $null; ScoopMs = $null; Ratio = $null; Note = 'network unavailable' })
        continue
    }

    $hok = Invoke-Tool $hokFull $c.args "$name-hok"
    $scoop = if ($scoopInvoke) { Invoke-Tool $scoopInvoke.Cmd ($scoopInvoke.Args + $c.args) "$name-scoop" } else { $null }

    if (-not $scoop) {
        $report.Add([pscustomobject]@{ Case = $name; Equiv = 'SKIP'; Hok = $hok.ExitCode; Scoop = $null; HokMs = [math]::Round($hok.Ms, 1); ScoopMs = $null; Ratio = $null; Note = 'scoop unavailable' })
        continue
    }

    $eqCode = $hok.ExitCode -eq $scoop.ExitCode
    $eqOut  = (Normalize $hok.Stdout) -eq (Normalize $scoop.Stdout)
    $equiv  = if ($hok.TimedOut -or $scoop.TimedOut) { 'TIMEOUT' } elseif ($eqCode -and $eqOut) { 'MATCH' } elseif ($eqCode) { 'OUTPUT-DIFF' } else { 'EXIT-DIFF' }
    $ratio  = if ($scoop.Ms -gt 0) { [math]::Round($hok.Ms / $scoop.Ms, 3) } else { $null }

    # Persist raw outputs for manual inspection.
    Set-Content -Path (Join-Path $OutputPath "$name.hok.out") -Value $hok.Stdout -Encoding utf8
    Set-Content -Path (Join-Path $OutputPath "$name.scoop.out") -Value $scoop.Stdout -Encoding utf8

    $report.Add([pscustomobject]@{
        Case    = $name
        Equiv   = $equiv
        Hok     = $hok.ExitCode
        Scoop   = $scoop.ExitCode
        HokMs   = [math]::Round($hok.Ms, 1)
        ScoopMs = [math]::Round($scoop.Ms, 1)
        Ratio   = $ratio
        Note    = $(if ($hok.TimedOut -or $scoop.TimedOut) { 'timed out (killed)' } else { $null })
    })
}

Write-Host ''
Write-Host ('hok   : ' + $(if ($hokFull) { $hokFull } else { 'N/A (not built)' }))
Write-Host ('scoop : ' + $(if ($scoopInvoke) { $scoopFull } else { 'N/A (not found)' }))
Write-Host ''
$report | Format-Table -AutoSize Case, Equiv, Hok, Scoop, HokMs, ScoopMs, Ratio, Note | Out-String -Width 120 | Write-Host

$report | Export-Csv -Path (Join-Path $OutputPath 'summary.csv') -NoTypeInformation
Write-Host "Detailed outputs saved to: $OutputPath"

$match = ($report | Where-Object Equiv -eq 'MATCH').Count
$diff  = ($report | Where-Object Equiv -in @('EXIT-DIFF', 'OUTPUT-DIFF')).Count
$skip  = ($report | Where-Object Equiv -in @('SKIP', 'TIMEOUT')).Count
Write-Host ("Result: {0} MATCH, {1} DIFF, {2} SKIP/TIMEOUT" -f $match, $diff, $skip)