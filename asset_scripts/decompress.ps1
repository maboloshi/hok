# Hok minimal helper — decompress functions for package script compatibility.
# Embedded in hok binary via include_str!.
#
# These functions do not decompress anything themselves: they emit a
# [[HOK_EXTRACT]] marker, and hok's Rust side performs the extraction natively
# (crates/libscoop/src/package/operations/extract.rs::extract_markers).
#
# Marker protocol — seven pipe-separated fields (a `|` cannot occur in a
# Windows path, so it is a safe separator):
#
#     format|source|dest|extract_dir|removal|switches|overwrite
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
# * `switches`    — the `-Switches` argument, space-joined. Official passes it
#                   on with `-split $Switches` (lib/decompress.ps1:99-101), so
#                   a space-joined field round-trips through a single split
#                   exactly like the original argument list. Empty when none.
# * `overwrite`   — the `-Overwrite` argument lowercased: `all` | `skip` |
#                   `rename` | empty. Only `Expand-7zipArchive` declares it
#                   (lib/decompress.ps1:77-79); official maps it to 7-Zip's
#                   `-aoa`/`-aos`/`-aou` (lib/decompress.ps1:102-106).
#
# The last two fields are *CLI* arguments of the helper tools, so the Rust side
# applies them by handing the marker to the real `7z.exe` / `innounp.exe` /
# `lessmsi.exe` instead of the native extractor — a marker without them keeps
# the native path unchanged. Silently dropping them is not an option: `-p`
# (encrypted archives), `-x!`/`-xr!` (exclusions) and `-Overwrite Skip|Rename`
# all change what ends up on disk.
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

function Write-ExtractMarker($format, $source, $dest, $extractDir, $removal, $switches, $overwrite) {
    $removalField = if ($removal) { 'removal' } else { '' }
    if ($null -eq $extractDir) { $extractDir = '' }
    # `-Switches` may arrive either as one argument holding several switches or
    # as several remaining arguments; joining with a space reproduces official's
    # `-split $Switches` for both shapes (PowerShell itself joins leftover
    # arguments with a space before official splits them again).
    $switchesField = if ($switches) { ($switches -join ' ').Trim() } else { '' }
    $overwriteField = if ($overwrite) { $overwrite.ToLowerInvariant() } else { '' }
    $marker = "[[HOK_EXTRACT]]$format|$source|$dest|$extractDir|$removalField|$switchesField|$overwriteField"
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
    # The explicit positions are load-bearing, and `ExtractDir` deliberately has
    # none. PowerShell makes a parameter positional only while *no* parameter of
    # the function declares a `Position`; once one does, every unpositioned
    # parameter is named-only and the leftovers go to the
    # `ValueFromRemainingArguments` parameter. Official Scoop positions exactly
    # `Path`/`DestinationPath` (`lib/decompress.ps1:66-71`), so in
    # `Expand-7zipArchive a b c` the `c` binds to `$Switches` — 7-Zip's own
    # option, which official then passes on with `-split $Switches`
    # (`lib/decompress.ps1:99-101`). Keeping `ExtractDir` positional at 2 (hok's
    # pre-X5b shape) instead swallowed `'-bsp1'`-style switches into a subtree
    # name that never exists, so the extraction silently produced an empty
    # directory. `-Overwrite` stays named-only for the same reason: a stray
    # positional must reach `$Switches`, not hit the ValidateSet.
    param(
        [Parameter(Position = 0)][String]$Path,
        [Parameter(Position = 1)][String]$DestinationPath = (Split-Path $Path),
        [String]$ExtractDir,
        [Parameter(ValueFromRemainingArguments = $true)][String[]]$Switches,
        [ValidateSet('All', 'Skip', 'Rename')][String]$Overwrite,
        [Switch]$Removal
    )
    Write-ExtractMarker "7z" $Path $DestinationPath $ExtractDir $Removal $Switches $Overwrite
}

function Expand-InnoArchive {
    # Same declaration shape as official `Expand-InnoArchive`
    # (`lib/decompress.ps1:229-242`).
    param(
        [Parameter(Position = 0)][String]$Path,
        [Parameter(Position = 1)][String]$DestinationPath = (Split-Path $Path),
        [String]$ExtractDir,
        [Parameter(ValueFromRemainingArguments = $true)][String[]]$Switches,
        [Switch]$Removal
    )
    Write-ExtractMarker "innosetup" $Path $DestinationPath $ExtractDir $Removal $Switches
}

function Expand-MsiArchive {
    # Same declaration shape as official `Expand-MsiArchive`
    # (`lib/decompress.ps1:172-185`); the remaining arguments reach lessmsi /
    # msiexec verbatim (`lib/decompress.ps1:199-201`).
    param(
        [Parameter(Position = 0)][String]$Path,
        [Parameter(Position = 1)][String]$DestinationPath = (Split-Path $Path),
        [String]$ExtractDir,
        [Parameter(ValueFromRemainingArguments = $true)][String[]]$Switches,
        [Switch]$Removal
    )
    Write-ExtractMarker "msi" $Path $DestinationPath $ExtractDir $Removal $Switches
}

# Distinct-name aliases (Scoop compatibility).
function Expand-Msi { Expand-MsiArchive @args }
function Expand-ZipArchive { Expand-7zipArchive @args }

function Expand-DarkArchive {
    # Same declaration shape as official `Expand-DarkArchive`
    # (`lib/decompress.ps1:306-316`).
    param(
        [Parameter(Position = 0)][String]$Path,
        [Parameter(Position = 1)][String]$DestinationPath = (Split-Path $Path),
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
