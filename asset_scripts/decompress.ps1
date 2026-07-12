# Hok minimal helper — decompress functions for package script compatibility.
# Embedded in hok binary via include_str!.
# P2: outputs [[HOK_EXTRACT]] markers for Rust native extraction.

function Write-ExtractMarker($format, $source, $dest, $removal) {
    $marker = "[[HOK_EXTRACT]]$format|$source|$dest"
    if ($removal) { $marker += "|removal" }
    Write-Host $marker
    # Also append to marker file for Rust side to read
    if ($env:HOK_EXTRACT_FILE) {
        $marker | Out-File -FilePath $env:HOK_EXTRACT_FILE -Append -Encoding ASCII
    }
}

function Expand-7zipArchive($Path, $DestinationPath, $ExtractDir, $Removal, $Switches) {
    $dest = if ($ExtractDir) { Join-Path $DestinationPath $ExtractDir } else { $DestinationPath }
    $null = New-Item -ItemType Directory -Path $dest -Force
    Write-ExtractMarker "7z" $Path $dest $Removal
    if ($Removal) { Remove-Item -Path $Path -Force -ErrorAction SilentlyContinue }
}

function Expand-InnoArchive($Path, $DestinationPath, $ExtractDir, $Removal, $Switches) {
    $dest = if ($ExtractDir) { Join-Path $DestinationPath $ExtractDir } else { $DestinationPath }
    $null = New-Item -ItemType Directory -Path $dest -Force
    Write-ExtractMarker "innosetup" $Path $dest $Removal
    if ($Removal) { Remove-Item -Path $Path -Force -ErrorAction SilentlyContinue }
}

function Expand-MsiArchive($Path, $DestinationPath, $ExtractDir, $Removal, $Switches) {
    $dest = if ($ExtractDir) { Join-Path $DestinationPath $ExtractDir } else { $DestinationPath }
    $null = New-Item -ItemType Directory -Path $dest -Force
    Write-ExtractMarker "msi" $Path $dest $Removal
    if ($Removal) { Remove-Item -Path $Path -Force -ErrorAction SilentlyContinue }
}

# Aliases (Scoop compatibility)
function Expand-7ZipArchive { Expand-7zipArchive @args }
function Expand-Msi { Expand-MsiArchive @args }
function Expand-ZipArchive { Expand-7zipArchive @args }
function Expand-DarkArchive { Expand-7zipArchive @args }
