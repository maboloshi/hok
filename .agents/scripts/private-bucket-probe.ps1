$ErrorActionPreference = 'Continue'
$hok = 'D:\App\github项目\Scoop\hok\target\debug\hok.exe'
$work = Join-Path $env:TEMP ("hok-priv-probe-" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work -Force | Out-Null
$realHome = $env:USERPROFILE

$tok = (gh auth token).Trim()
$cands = @(gh api '/user/repos?visibility=private&per_page=5' --jq '.[].full_name')
$repo = $cands[0].Trim()
$repo2 = ($cands | Select-Object -Last 1).Trim()
Write-Output ("REPO=" + $repo)

function New-Root([string]$name) {
    $r = Join-Path $work $name
    New-Item -ItemType Directory -Path $r -Force | Out-Null
    return $r
}
function Show-Tree([string]$root) {
    $b = Join-Path $root 'buckets'
    if (Test-Path $b) {
        Get-ChildItem $b -Force | ForEach-Object {
            $inner = @(Get-ChildItem $_.FullName -Force -ErrorAction SilentlyContinue).Count
            '  leftover: ' + $_.Name + ' entries=' + $inner
        }
    } else { '  (no buckets dir)' }
}
function Run-Hok([string]$root, [string]$name, [string]$url) {
    $env:SCOOP = $root
    $o = & $hok bucket add $name $url 2>&1 | Out-String
    Write-Output ("exit=" + $LASTEXITCODE)
    Write-Output ($o.Trim() -split "`n" | Select-Object -Last 5)
    Show-Tree $root
}

# --- T0: HTTPS private, no credential helper (empty temp HOME) --------------------
Write-Output '=== T0 https private, no helper ==='
$emptyHome = Join-Path $work 'emptyhome'; New-Item -ItemType Directory -Path $emptyHome -Force | Out-Null
$env:HOME = $emptyHome; $env:USERPROFILE = $emptyHome
Run-Hok (New-Root 'r0') 'privt0' "https://github.com/$repo.git"

# --- T1: HTTPS private, credential.helper from temp gitconfig ---------------------
Write-Output '=== T1 https private + credential.helper ==='
$helperHome = Join-Path $work 'helperhome'; New-Item -ItemType Directory -Path $helperHome -Force | Out-Null
$cfg = "[credential `"https://github.com`"]`n`thelper = `"!f() { echo username=x-access-token; echo password=$tok; }; f`"`n"
Set-Content -Path (Join-Path $helperHome '.gitconfig') -Value $cfg -Encoding ascii
$env:HOME = $helperHome; $env:USERPROFILE = $helperHome
$root1 = New-Root 'r1'
Run-Hok $root1 'privt1' "https://github.com/$repo.git"
Write-Output ('  buckets cloned: ' + @(Get-ChildItem (Join-Path $root1 'buckets') -Force -ErrorAction SilentlyContinue).Count)

# --- T2: HTTPS private, real global config (GCM), non-interactive -----------------
Write-Output '=== T2 https private + real GCM (non-interactive) ==='
$env:HOME = $realHome; $env:USERPROFILE = $realHome
$env:GCM_INTERACTIVE = 'never'; $env:GCM_PROVIDER = 'github'
Run-Hok (New-Root 'r2') 'privt2' "https://github.com/$repo.git"
Remove-Item Env:GCM_INTERACTIVE -ErrorAction SilentlyContinue
Remove-Item Env:GCM_PROVIDER -ErrorAction SilentlyContinue

# --- T3: HTTPS with token embedded in URL ----------------------------------------
Write-Output '=== T3 https with token in URL ==='
$root3 = New-Root 'r3'
Run-Hok $root3 'privt3' "https://x-access-token:$tok@github.com/$repo.git"
$storedCfg = Join-Path $root3 'buckets\privt3\.git\config'
if (Test-Path $storedCfg) {
    Write-Output ('  stored url contains token: ' + (Select-String -Path $storedCfg -Pattern 'x-access-token' -Quiet))
}
$env:SCOOP = $root3
$listOut = & $hok bucket list 2>&1 | Out-String
Write-Output ('  bucket list leaks token: ' + ($listOut -match 'x-access-token'))

# --- T4: SSH URL ------------------------------------------------------------------
Write-Output '=== T4 ssh url ==='
Run-Hok (New-Root 'r4') 'privt4' "git@github.com:$repo.git"

# --- T5: public bucket still fine (control) ---------------------------------------
Write-Output '=== T5 control: public bucket ==='
Run-Hok (New-Root 'r5') 'main' 'https://github.com/ScoopInstaller/Main.git'

Remove-Item Env:SCOOP -ErrorAction SilentlyContinue
$env:HOME = $realHome; $env:USERPROFILE = $realHome
Write-Output ("WORKDIR=" + $work)
