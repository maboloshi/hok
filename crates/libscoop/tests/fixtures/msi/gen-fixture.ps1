# Generates `x5b-fixture.msi`: a real, minimal Windows Installer package used by
# `operations::script::tests::expand_msi_archive_switches_reach_lessmsi`.
#
# Tools: only what Windows ships — the `WindowsInstaller.Installer` COM
# automation API and `makecab.exe`. No network, no third-party authoring tool.
#
# Layout the fixture is built for (as lessmsi sees it):
#
#     SourceDir\X5bFixture\hello.txt
#     SourceDir\X5bFixture\other.txt
#
# Two payload files are what make `-Switches '<name>'` observable: lessmsi
# treats a trailing bare argument as a file-name filter, so a correct run of the
# helper extracts only the named file.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File gen-fixture.ps1
param(
    [string]$OutDir = $PSScriptRoot,
    # `makecab.exe` is an ANSI program: a non-ASCII scratch path (this repo's
    # checkout, or a user profile with non-ASCII characters) makes it fail with
    # "Path is invalid". The cabinet is therefore staged under an ASCII path and
    # only the finished MSI is written to `$OutDir`.
    [string]$Scratch = "$env:SystemDrive\x5b-msi-build"
)
$ErrorActionPreference = 'Stop'

$stage = Join-Path $Scratch 'stage'
Remove-Item $Scratch -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $stage -Force | Out-Null
Set-Content -Path (Join-Path $stage 'hello.txt') -Value 'hello from x5b msi fixture' -Encoding ascii -NoNewline
Set-Content -Path (Join-Path $stage 'other.txt') -Value 'other payload' -Encoding ascii -NoNewline

# ── cabinet ────────────────────────────────────────────────────────────────
$cab = Join-Path $Scratch 'payload.cab'
$ddf = Join-Path $stage 'payload.ddf'
@"
.OPTION EXPLICIT
.Set CabinetNameTemplate=payload.cab
.Set DiskDirectoryTemplate=$Scratch
.Set CompressionType=MSZIP
.Set Cabinet=on
.Set Compress=on
"hello.txt" "hello.txt"
"other.txt" "other.txt"
"@ | Set-Content -Path $ddf -Encoding ascii
Push-Location $stage
try { & makecab.exe /f $ddf | Out-Null } finally { Pop-Location }
if (-not (Test-Path $cab)) { throw "makecab did not produce $cab" }

$helloSize = (Get-Item (Join-Path $stage 'hello.txt')).Length
$otherSize = (Get-Item (Join-Path $stage 'other.txt')).Length

# ── database ───────────────────────────────────────────────────────────────
$msi = Join-Path $OutDir 'x5b-fixture.msi'
Remove-Item $msi -Force -ErrorAction SilentlyContinue
$installer = New-Object -ComObject WindowsInstaller.Installer
$db = $installer.OpenDatabase($msi, 3) # msiOpenDatabaseModeCreate

function Invoke-Sql([string]$Sql, $Record) {
    $view = $db.OpenView($Sql)
    if ($Record) { $view.Execute($Record) } else { $view.Execute() }
    $view.Close()
}

$ddl = @(
    'CREATE TABLE `Property` (`Property` CHAR(72) NOT NULL, `Value` CHAR(0) NOT NULL PRIMARY KEY `Property`)',
    'CREATE TABLE `Directory` (`Directory` CHAR(72) NOT NULL, `Directory_Parent` CHAR(72), `DefaultDir` CHAR(255) NOT NULL PRIMARY KEY `Directory`)',
    'CREATE TABLE `Component` (`Component` CHAR(72) NOT NULL, `ComponentId` CHAR(38), `Directory_` CHAR(72) NOT NULL, `Attributes` SHORT NOT NULL, `Condition` CHAR(255), `KeyPath` CHAR(72) PRIMARY KEY `Component`)',
    'CREATE TABLE `Feature` (`Feature` CHAR(38) NOT NULL, `Feature_Parent` CHAR(38), `Title` CHAR(64), `Description` CHAR(255), `Display` SHORT, `Level` SHORT NOT NULL, `Directory_` CHAR(72), `Attributes` SHORT NOT NULL PRIMARY KEY `Feature`)',
    'CREATE TABLE `FeatureComponents` (`Feature_` CHAR(38) NOT NULL, `Component_` CHAR(72) NOT NULL PRIMARY KEY `Feature_`, `Component_`)',
    'CREATE TABLE `Media` (`DiskId` SHORT NOT NULL, `LastSequence` SHORT NOT NULL, `DiskPrompt` CHAR(64), `Cabinet` CHAR(255), `VolumeLabel` CHAR(32), `Source` CHAR(72) PRIMARY KEY `DiskId`)',
    'CREATE TABLE `File` (`File` CHAR(72) NOT NULL, `Component_` CHAR(72) NOT NULL, `FileName` CHAR(255) NOT NULL, `FileSize` LONG NOT NULL, `Version` CHAR(72), `Language` CHAR(20), `Attributes` SHORT, `Sequence` SHORT NOT NULL PRIMARY KEY `File`)'
)
foreach ($s in $ddl) { Invoke-Sql $s }

$productCode = '{A1B2C3D4-1111-4222-8333-444455556666}'
$upgradeCode = '{A1B2C3D4-2222-4222-8333-444455556666}'
$cmpHello = '{A1B2C3D4-3333-4222-8333-444455556666}'
$cmpOther = '{A1B2C3D4-4444-4222-8333-444455556666}'

$rows = @(
    "INSERT INTO ``Property`` (``Property``, ``Value``) VALUES ('ProductCode', '$productCode')",
    "INSERT INTO ``Property`` (``Property``, ``Value``) VALUES ('ProductName', 'x5b-msi-fixture')",
    "INSERT INTO ``Property`` (``Property``, ``Value``) VALUES ('ProductVersion', '1.0.0')",
    "INSERT INTO ``Property`` (``Property``, ``Value``) VALUES ('Manufacturer', 'hok')",
    "INSERT INTO ``Property`` (``Property``, ``Value``) VALUES ('ProductLanguage', '1033')",
    "INSERT INTO ``Property`` (``Property``, ``Value``) VALUES ('UpgradeCode', '$upgradeCode')",
    "INSERT INTO ``Property`` (``Property``, ``Value``) VALUES ('ALLUSERS', '1')",
    "INSERT INTO ``Directory`` (``Directory``, ``Directory_Parent``, ``DefaultDir``) VALUES ('TARGETDIR', NULL, 'SourceDir')",
    "INSERT INTO ``Directory`` (``Directory``, ``Directory_Parent``, ``DefaultDir``) VALUES ('INSTALLDIR', 'TARGETDIR', 'X5bFixture')",
    "INSERT INTO ``Component`` (``Component``, ``ComponentId``, ``Directory_``, ``Attributes``, ``Condition``, ``KeyPath``) VALUES ('CmpHello', '$cmpHello', 'INSTALLDIR', 0, NULL, 'hello.txt')",
    "INSERT INTO ``Component`` (``Component``, ``ComponentId``, ``Directory_``, ``Attributes``, ``Condition``, ``KeyPath``) VALUES ('CmpOther', '$cmpOther', 'INSTALLDIR', 0, NULL, 'other.txt')",
    "INSERT INTO ``Feature`` (``Feature``, ``Feature_Parent``, ``Title``, ``Description``, ``Display``, ``Level``, ``Directory_``, ``Attributes``) VALUES ('Feature1', NULL, 'x5b fixture', 'hok msi fixture', 1, 1, 'INSTALLDIR', 0)",
    "INSERT INTO ``FeatureComponents`` (``Feature_``, ``Component_``) VALUES ('Feature1', 'CmpHello')",
    "INSERT INTO ``FeatureComponents`` (``Feature_``, ``Component_``) VALUES ('Feature1', 'CmpOther')",
    # The leading `#` is what tells the installer (and lessmsi) that the cabinet
    # is embedded as a stream inside the MSI rather than sitting next to it;
    # without it lessmsi reports `ExternalCabNotFoundException`.
    "INSERT INTO ``Media`` (``DiskId``, ``LastSequence``, ``DiskPrompt``, ``Cabinet``, ``VolumeLabel``, ``Source``) VALUES (1, 2, NULL, '#payload.cab', NULL, NULL)",
    "INSERT INTO ``File`` (``File``, ``Component_``, ``FileName``, ``FileSize``, ``Version``, ``Language``, ``Attributes``, ``Sequence``) VALUES ('hello.txt', 'CmpHello', 'hello.txt', $helloSize, NULL, NULL, 0, 1)",
    "INSERT INTO ``File`` (``File``, ``Component_``, ``FileName``, ``FileSize``, ``Version``, ``Language``, ``Attributes``, ``Sequence``) VALUES ('other.txt', 'CmpOther', 'other.txt', $otherSize, NULL, NULL, 0, 2)"
)
foreach ($s in $rows) { Invoke-Sql $s }

# ── embedded cabinet stream ────────────────────────────────────────────────
$rec = $installer.CreateRecord(2)
$rec.StringData(1) = 'payload.cab'
$rec.SetStream(2, $cab)
Invoke-Sql 'INSERT INTO `_Streams` (`Name`, `Data`) VALUES (?, ?)' $rec

# ── summary information ────────────────────────────────────────────────────
$si = $db.SummaryInformation(20)
$si.Property(2) = 'Installation Database'
$si.Property(3) = 'x5b msi fixture'
$si.Property(4) = 'hok'
$si.Property(7) = 'Intel;1033'
$si.Property(9) = $productCode
$si.Persist()

$db.Commit()
$db = $null
[System.Runtime.InteropServices.Marshal]::ReleaseComObject($installer) | Out-Null
[GC]::Collect()
[GC]::WaitForPendingFinalizers()
Remove-Item $Scratch -Recurse -Force -ErrorAction SilentlyContinue

# The Windows Installer service finishes writing the database asynchronously,
# so do not report size or hash from inside this process — validate the result
# with `lessmsi x <msi> <dir>\` once the process has exited.
Write-Host "wrote $msi"
