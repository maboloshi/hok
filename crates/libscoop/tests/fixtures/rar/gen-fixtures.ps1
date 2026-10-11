<#
.SYNOPSIS
  Regenerate the WinRAR-produced part of the RAR fixture corpus.

.DESCRIPTION
  Produces deterministic *content* (an LCG plus fixed text) and archives it with
  WinRAR's CLI. The archive container itself is not byte-reproducible because
  WinRAR stamps host OS / timestamp / quick-open fields, so the tests assert
  content (relative path set + per-file bytes), never archive bytes.

  The test suite never runs this script: CI has no WinRAR and no network. The
  archives are committed instead. Re-run only when the corpus changes.

  RAR4 is deliberately absent here. RAR 7.23 removed RAR4 *creation*
  (`-ma4` -> `ERROR: Unknown option: ma4`, exit code 7), so RAR4 coverage comes
  from the vendored `rars-*` fixtures instead; see README.md.

.NOTES
  Exact toolchain that produced the committed archives:

    RAR 7.23 x64   (WinRAR 7.23, Trial)
    D:\App\Scoop\apps\winrar\current\Rar.exe

  Every archive produced here is checked against the old backend (`unrar` 0.5)
  by `rar_oracle_*` tests, and the raw invocation for each is echoed at the end
  of a run so README.md can be kept in sync.
#>
[CmdletBinding()]
param(
    [string]$RarExe = 'D:\App\Scoop\apps\winrar\current\Rar.exe',
    [string]$OutDir = $PSScriptRoot,
    [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\..\..')).Path
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path $RarExe)) { throw "Rar.exe not found: $RarExe" }

$work = Join-Path $env:TEMP 'hok-rar-fixtures-work'
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Force -Path $work | Out-Null

# --- deterministic content ------------------------------------------------------
$gen = @'
const fs = require('node:fs');
const path = require('node:path');
const root = process.argv[2];
const w = (rel, buf) => {
  const p = path.join(root, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, buf);
};
// Deterministic LCG so the corpus is reproducible without checking in blobs.
// `Math.imul` keeps the multiply exact in 32 bits: a plain `seed * 1103515245`
// exceeds 2^53 and rounds, which makes the corpus depend on JS float behaviour.
// The Rust generator (examples/gen_rar_fixtures.rs) uses the same recurrence, so
// both produce byte-identical payloads.
const lcg = (seed) => () => {
  seed = (Math.imul(seed, 1103515245) + 12345) & 0x7fffffff;
  return (seed >>> 8) & 0xff;
};
const bytes = (n, seed) => { const f = lcg(seed); return Buffer.from(Array.from({ length: n }, f)); };

const base = path.join(root, 'base');
const readme = Array.from({ length: 200 }, (_, i) =>
  `line ${String(i).padStart(4, '0')}: the quick brown fox jumps over the lazy dog\n`).join('');
w('base/readme.txt', Buffer.from(readme, 'utf8'));
w('base/empty.bin', Buffer.alloc(0));
w('base/binary.bin', bytes(4096, 12345));
w('base/sub/nested.txt', Buffer.from('nested payload\n'.repeat(64), 'utf8'));
w('base/sub/deep/leaf.bin', bytes(512, 777));
w('base/\u4e2d\u6587\u540d/\u6d4b\u8bd5-\u6587\u4ef6.txt', Buffer.from('non-ASCII path and content\n', 'utf8'));
w('base/emoji-\ud83d\ude00.txt', Buffer.from('emoji name\n', 'utf8'));
void base;

// Incompressible payload for split volumes; stored (-m0) so a volume boundary
// lands inside a member and the continuation path is exercised.
w('vol/volpayload.bin', bytes(40 * 1024, 424242));

w('vol/volpayload.bin', bytes(40 * 1024, 424242));
'@
$genFile = Join-Path $work 'gen.cjs'
Set-Content -Path $genFile -Value $gen -Encoding utf8
& node $genFile $work
if ($LASTEXITCODE -ne 0) { throw 'content generation failed' }

# The x86-filter fixture needs a payload WinRAR's filter heuristic actually
# accepts. A hand-built PE does not qualify — measured: `-mcx+` and `-mcx-`
# both give 253 bytes for the synthetic buffer above, i.e. no filter ran. A
# real PE does qualify (hok-shim.exe: 5344 vs 5384 bytes). So the real one is
# used, and it is our own build artifact, which keeps the licence clean.
$prog = Join-Path $work 'prog\prog.exe'
New-Item -ItemType Directory -Force -Path (Split-Path $prog) | Out-Null
$shim = Join-Path $RepoRoot 'target\release\hok-shim.exe'
if (-not (Test-Path $shim)) {
    Write-Host "building hok-shim (needed as the x86-filter payload)..."
    Push-Location $RepoRoot
    try { & cargo build -p hok-shim --release | Out-Null } finally { Pop-Location }
}
if (-not (Test-Path $shim)) { throw "x86-filter payload not found: $shim" }
Copy-Item -Force $shim $prog
Write-Host "x86-filter payload: $shim ($((Get-Item $prog).Length) bytes)"

# --- archive matrix -------------------------------------------------------------
$invocations = [System.Collections.Generic.List[string]]::new()

function New-Rar {
    param([string]$Name, [string[]]$Switches, [string[]]$SubDir)
    $out = Join-Path $OutDir $Name
    # Clear a previous volume set: `rar5-vol.rar` -> `rar5-vol.part*.rar`.
    $stem = $Name -replace '\.part1\.rar$', '.part*' -replace '\.rar$', '.part*'
    Get-ChildItem -Path $OutDir -Filter $stem -File -ErrorAction SilentlyContinue | Remove-Item -Force
    $argv = @('a') + $Switches + @($out) + @('*')
    Push-Location (Join-Path $work $SubDir)
    try { & $RarExe @argv | Out-Null; $code = $LASTEXITCODE }
    finally { Pop-Location }
    if ($code -ne 0) { throw "rar failed ($code): $($argv -join ' ')" }
    $invocations.Add("Rar.exe $($argv -join ' ')")
}

New-Rar 'rar5-normal.rar'    @('-ma5', '-m3', '-ep1', '-r', '-mt1')            'base'
New-Rar 'rar5-solid.rar'     @('-ma5', '-m5', '-s', '-ep1', '-r', '-mt1')      'base'
New-Rar 'rar5-store.rar'     @('-ma5', '-m0', '-ep1', '-r', '-mt1')            'base'
New-Rar 'rar5-x86filter.rar' @('-ma5', '-m5', '-mcx+', '-qo-', '-ep1', '-r', '-mt1') 'prog'
New-Rar 'rar5-encrypted.rar' @('-ma5', '-m3', '-pSECRET', '-ep1', '-r', '-mt1') 'base'
New-Rar 'rar5-vol.rar'        @('-ma5', '-m0', '-v16k', '-ep1', '-r', '-mt1')   'vol'

# Prove the x86 filter actually ran: with `-mcx-` the same payload must produce
# a larger archive. Both variants are built here with otherwise identical flags
# (including `-qo-`, whose quick-open record is pure overhead), so the size gap
# measures the filter and nothing else. If these ever match, rar5-x86filter.rar
# stopped covering the filter and the payload must be replaced, not silently kept.
$engage = @('-ma5', '-m5', '-ep1', '-r', '-mt1', '-qo-')
Push-Location (Join-Path $work 'prog')
try {
    & $RarExe a @engage '-mcx+' (Join-Path $work 'engage-on.rar') prog.exe | Out-Null
    & $RarExe a @engage '-mcx-' (Join-Path $work 'engage-off.rar') prog.exe | Out-Null
}
finally { Pop-Location }
$withFilter = (Get-Item (Join-Path $work 'engage-on.rar')).Length
$withoutFilter = (Get-Item (Join-Path $work 'engage-off.rar')).Length
if ($withFilter -ge $withoutFilter) {
    throw "x86 filter did not engage: -mcx+ = $withFilter, -mcx- = $withoutFilter"
}
Write-Host "x86 filter engaged: -mcx+ = $withFilter < -mcx- = $withoutFilter"

# --- derived: truncated archive (must fail cleanly, never panic) -----------------
$src = Join-Path $OutDir 'rar5-normal.rar'
$all = [IO.File]::ReadAllBytes($src)
$cut = [int]($all.Length / 2)
[IO.File]::WriteAllBytes((Join-Path $OutDir 'rar5-truncated.rar'), $all[0..($cut - 1)])

Write-Host "`nRar.exe invocations (keep README.md in sync):"
$invocations | ForEach-Object { Write-Host "  $_" }
Write-Host "  (rar5-truncated.rar = first $cut bytes of rar5-normal.rar)"
