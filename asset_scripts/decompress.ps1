# Hok minimal helper — decompress functions for package script compatibility.
# Embedded in hok binary via include_str!.
#
# These functions do not decompress anything themselves: they emit a
# [[HOK_EXTRACT]] marker, and hok's Rust side performs the extraction natively
# (crates/libscoop/src/package/operations/extract.rs::extract_markers).
#
# Marker protocol — five pipe-separated fields (a `|` cannot occur in a
# Windows path, so it is a safe separator):
#
#     format|source|dest|extract_dir|removal
#
# * `format`      — `7z` | `innosetup` | `msi`; selects the Rust extractor.
# * `source`      — absolute path of the archive to extract.
# * `dest`        — the *base* destination directory, NOT pre-joined with
#                   `extract_dir`: the Rust side applies `extract_dir` with
#                   Scoop's semantics (extract only that subtree, then hoist
#                   its contents up into `dest`; see `Expand-7zipArchive` in
#                   official Scoop `lib/decompress.ps1`).
# * `extract_dir` — the `ExtractDir` argument, or empty.
# * `removal`     — the literal `removal` when `-Removal` was requested, else
#                   empty. The Rust side deletes `source` only *after* a
#                   successful extraction, so a failed extraction never loses
#                   the archive.
#
# Do NOT add a case-variant wrapper of an existing function here. PowerShell
# function names are case-insensitive, so `function Expand-7ZipArchive { ... }`
# does not alias `Expand-7zipArchive` — it *replaces* it. A wrapper whose body
# calls the name it just replaced recurses until "call depth overflow", which
# is exactly what the deleted line here used to do. The distinct names below
# (`Expand-Msi`, `Expand-ZipArchive`) are fine: they differ by more than case.
#
# Every function's `$DestinationPath` defaults to the archive's own directory,
# mirroring official Scoop (`lib/decompress.ps1:71,177,229,305`). Official then
# applies `$DestinationPath.TrimEnd('\')` because it hands the value to
# `7z -o`; hok deliberately does NOT, because for a drive-root archive path
# (`C:\pkg.7z`) that rewrite yields the drive-relative `C:` instead of `C:\`,
# and the Rust side normalizes the path anyway.

function Write-ExtractMarker($format, $source, $dest, $extractDir, $removal) {
    $removalField = if ($removal) { 'removal' } else { '' }
    if ($null -eq $extractDir) { $extractDir = '' }
    $marker = "[[HOK_EXTRACT]]$format|$source|$dest|$extractDir|$removalField"
    Write-Host $marker
    # Append to the marker file for the Rust side to read. UTF-8 (not ASCII):
    # a source/destination under a non-ASCII path would otherwise be written
    # as `?`, and the Rust side would then skip the extraction because the
    # mangled source does not exist. PowerShell 5.1 prefixes this file with a
    # UTF-8 BOM on creation; the Rust reader strips it.
    if ($env:HOK_EXTRACT_FILE) {
        $marker | Out-File -FilePath $env:HOK_EXTRACT_FILE -Append -Encoding UTF8
    }
}

function Expand-7zipArchive {
    param(
        [String]$Path,
        [String]$DestinationPath = (Split-Path $Path),
        [String]$ExtractDir,
        [Parameter(ValueFromRemainingArguments = $true)][String[]]$Switches,
        [Switch]$Removal
    )
    Write-ExtractMarker "7z" $Path $DestinationPath $ExtractDir $Removal
}

function Expand-InnoArchive {
    param(
        [String]$Path,
        [String]$DestinationPath = (Split-Path $Path),
        [String]$ExtractDir,
        [Parameter(ValueFromRemainingArguments = $true)][String[]]$Switches,
        [Switch]$Removal
    )
    Write-ExtractMarker "innosetup" $Path $DestinationPath $ExtractDir $Removal
}

function Expand-MsiArchive {
    param(
        [String]$Path,
        [String]$DestinationPath = (Split-Path $Path),
        [String]$ExtractDir,
        [Parameter(ValueFromRemainingArguments = $true)][String[]]$Switches,
        [Switch]$Removal
    )
    Write-ExtractMarker "msi" $Path $DestinationPath $ExtractDir $Removal
}

# Distinct-name aliases (Scoop compatibility).
function Expand-Msi { Expand-MsiArchive @args }
function Expand-ZipArchive { Expand-7zipArchive @args }

function Expand-DarkArchive {
    param(
        [String]$Path,
        [String]$DestinationPath = (Split-Path $Path),
        [String]$ExtractDir,
        [Parameter(ValueFromRemainingArguments = $true)][String[]]$Switches,
        [Switch]$Removal
    )
    $dest = if ($ExtractDir) { Join-Path $DestinationPath $ExtractDir } else { $DestinationPath }
    $null = New-Item -ItemType Directory -Path $dest -Force
    $DarkPath = Get-HelperPath -Helper Dark
    if ((Split-Path $DarkPath -Leaf) -eq 'wix.exe') {
        $ArgList = @('burn', 'extract', $Path, '-out', $dest, '-outba', "$dest\UX")
    } else {
        $ArgList = @('-nologo', '-x', $dest, $Path)
    }
    if ($Switches) { $ArgList += $Switches }
    Invoke-ExternalCommand $DarkPath $ArgList
    if ($Removal) { Remove-Item -Path $Path -Force -ErrorAction SilentlyContinue }
}
