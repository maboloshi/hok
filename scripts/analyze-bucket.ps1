# analyze-bucket.ps1
# 扫描 bucket 目录的 manifest，提取包脚本中调用的 Scoop helper 函数
# 用法: .\scripts\analyze-bucket.ps1 -Path D:\App\Scoop\buckets\scoop-private\bucket
#       .\scripts\analyze-bucket.ps1 -Path main,bucket\extra -Root D:\Scoop\buckets

param(
    [Parameter(Mandatory = $true)]
    [string[]]$Path,

    # 当 Path 是 bucket 名称列表时，从此根目录拼接
    [string]$Root,

    [switch]$ShowDetail,

    # 已知函数列表文件（每行一个函数名），匹配到的不再高亮
    [string]$KnownFunctionsFile
)

$knownFunctions = @()
if ($KnownFunctionsFile -and (Test-Path $KnownFunctionsFile)) {
    $knownFunctions = Get-Content $KnownFunctionsFile | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne '' }
}

# Scoop 特有 helper 函数（大小写敏感精确匹配）
# 这些是 Scoop lib/*.ps1 中定义的、包脚本会直接调用的函数
$scoopHelpers = @(
    'Expand-7zipArchive', 'Expand-7ZipArchive', 'Expand-ZipArchive',
    'Expand-InnoArchive', 'Expand-MsiArchive', 'Expand-DarkArchive',
    'Get-HelperPath', 'Get-InstalledVersion', 'Get-Version',
    'Select-CurrentVersion',
    'New-DirectoryJunction',
    'Invoke-ExternalCommand',
    'is_admin',
    'ensure',
    'info', 'warn', 'error',
    'friendly_path', 'abort',
    'WriteReg', 'Get-RegKey', 'Remove-RegKey', 'New-RegKey',
    'Write-Env', 'Remove-Env',
    'New-Update', 'New-IssueMsg',
    'Get-InstallPath', 'Get-AppDir',
    'Install-Module', 'Uninstall-Module',
    'New-InstallUpdate',
    'Get-InstalledApps',
    'Format-Message',
    'Stop-Service', 'Start-Service',
    'Remove-ItemIfExists',
    'Expand-Msi'  # 别名
)

$scriptExtensions = @('pre_install', 'post_install', 'pre_uninstall', 'post_uninstall')
$scriptBlocks = @('installer.script', 'uninstaller.script')

# 解析所有路径
$resolvedPaths = @()
foreach ($p in $Path) {
    if ($Root) {
        $full = Join-Path $Root $p
        if (!(Test-Path $full)) { Write-Host "WARN: $full not found" -ForegroundColor Yellow; continue }
        # Scoop buckets have a 'bucket' subdirectory containing manifests
        $manifestDir = Join-Path $full "bucket"
        if (Test-Path $manifestDir) { $resolvedPaths += $manifestDir }
        else { $resolvedPaths += $full }
    } else {
        if (Test-Path $p) { $resolvedPaths += $p }
        else { Write-Host "WARN: $p not found" -ForegroundColor Yellow }
    }
}

if ($resolvedPaths.Count -eq 0) { Write-Host "ERROR: no valid paths" -ForegroundColor Red; exit 1 }

Write-Host "`nScanning: $($resolvedPaths -join ', ')`n" -ForegroundColor Cyan

$results = @{}
$scriptManifests = 0

foreach ($scanPath in $resolvedPaths) {
    if ($ShowDetail) { Write-Host "[$scanPath]" -ForegroundColor Cyan }

$manifests = Get-ChildItem "$scanPath\*.json"
$totalManifests = $manifests.Count

foreach ($file in $manifests) {
    try {
        $json = Get-Content $file.FullName -Raw -Encoding UTF8 -ErrorAction Stop | ConvertFrom-Json
    } catch { continue }

    $hasScript = $false
    $manifestName = $file.BaseName
    $foundHere = @{}

    # 提取所有脚本文本
    $allScriptText = ''

    foreach ($key in $scriptExtensions) {
        $lines = $json.$key
        if ($lines -and $lines.Count -gt 0) {
            $hasScript = $true
            $allScriptText += "`n" + ($lines -join "`n")
        }
    }

    foreach ($key in $scriptBlocks) {
        $block = $json.$key
        if ($block -and $block.script -and $block.script.Count -gt 0) {
            $hasScript = $true
            $allScriptText += "`n" + ($block.script -join "`n")
        }
    }

    # 在脚本文本中精确匹配已知 helper 函数名
    # 只匹配作为命令使用的场景（前面是行首、管道、分号、括号等）
    foreach ($helper in $scoopHelpers) {
        # 转义函数名中的特殊字符用于 regex
        $escaped = [regex]::Escape($helper)
        # 匹配作为命令出现的函数名（前面不是变量引用)
        $pattern = "(?<![-\$\\w])$escaped(?![\\w-])"
        if ($allScriptText -match $pattern) {
            $foundHere[$helper] = $true
        }
    }

    if ($ShowDetail -and $hasScript) {
        if ($foundHere.Keys.Count -gt 0) {
            Write-Host "  [$manifestName] $($foundHere.Keys -join ', ')" -ForegroundColor Yellow
        } elseif ($ShowDetail) {
            Write-Host "  [$manifestName] (scripts, no helpers)" -ForegroundColor DarkGray
        }
    }

    if ($hasScript) { $scriptManifests++ }
    foreach ($cmd in $foundHere.Keys) {
        if (!$results.ContainsKey($cmd)) {
            $results[$cmd] = @{ Count = 0; Manifests = @() }
        }
        $results[$cmd].Count++
        $results[$cmd].Manifests += $manifestName
    }
}  # foreach file
}  # foreach scanPath

# 计算总清单数
$allManifests = 0
foreach ($path in $resolvedPaths) {
    $allManifests += (Get-ChildItem "$path\*.json").Count
}

Write-Host "`n========== Summary ==========" -ForegroundColor Cyan
Write-Host "Paths scanned : $($resolvedPaths.Count)"
Write-Host "Total manifests : $allManifests"
Write-Host "With scripts   : $scriptManifests"
Write-Host "With helpers   : $($results.Keys.Count) unique functions`n"

if ($results.Keys.Count -eq 0) { Write-Host "No Scoop helper calls found." -ForegroundColor Yellow; exit 0 }

$sorted = $results.GetEnumerator() | Sort-Object { $_.Value.Count } -Descending

Write-Host ("{0,-30} {1,6}  {2}" -f "Function", "Count", "Manifests") -ForegroundColor Green
Write-Host ("{0,-30} {1,6}  {2}" -f "--------", "-----", "---------")
foreach ($entry in $sorted) {
    $mStr = $entry.Value.Manifests -join ', '
    if ($mStr.Length -gt 80) { $mStr = $mStr.Substring(0, 77) + '...' }
    Write-Host ("{0,-30} {1,6}  {2}" -f $entry.Key, $entry.Value.Count, $mStr)
}

# 未知函数（已知列表以外的）
if ($knownFunctions.Count -gt 0) {
    $unknown = $sorted | Where-Object { $_.Key -notin $knownFunctions }
    if ($unknown) {
        Write-Host "`n===== Unknown (not in known list) =====" -ForegroundColor Yellow
        foreach ($entry in $unknown) {
            Write-Host "  $($entry.Key) ($($entry.Value.Count) times)" -ForegroundColor Red
        }
    } else {
        Write-Host "`nAll found functions are covered in the known list." -ForegroundColor Green
    }
}

Write-Host "`nDone.`n" -ForegroundColor Cyan
