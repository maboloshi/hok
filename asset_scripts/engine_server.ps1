# hok persistent PowerShell engine (server side).
#
# Kept alive by PsEngine (package/operations/psengine.rs) and driven over
# stdin. It reads one submission frame per line:
#
#     RUN <json>
#
# where <json> is a single-line object:
#     { "t": token, "p": script path, "env": {NAME: value, ...}, "f": out file }
#
# For each frame it applies env to the *process* environment, runs the script
# at p, capturing its full output (all streams, merged) into the file at f,
# then reports completion on stdout with:
#
#     DONE_<token>:<exitcode>
#
# The exitcode is the script's LASTEXITCODE (0 when none). A thrown error
# appends a marker line to the capture file and yields exitcode 1. QUIT on
# its own line stops the loop.
#
# Output goes to a per-submission capture file rather than this process's
# stdout so a single shared engine can multiplex submissions without hok
# needing to split its stdout by token, and a chatty script can never fill a
# pipe and deadlock the worker.
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

while (($line = [Console]::In.ReadLine()) -ne $null) {
    $line = $line.Trim()
    if ($line -eq '') { continue }
    if ($line -eq 'QUIT') { break }
    if (-not $line.StartsWith('RUN ')) { continue }

    try {
        $o = $line.Substring(4) | ConvertFrom-Json
    } catch {
        [Console]::Out.WriteLine('DONE_BADJSON:2')
        [Console]::Out.Flush()
        continue
    }

    $token = [string]$o.t
    $path = [string]$o.p
    $outFile = [string]$o.f

    if ($o.env) {
        foreach ($prop in $o.env.PSObject.Properties) {
            [Environment]::SetEnvironmentVariable($prop.Name, [string]$prop.Value)
        }
    }

    if (-not (Test-Path -LiteralPath $path)) {
        [Console]::Out.WriteLine('DONE_' + $token + ':127')
        [Console]::Out.Flush()
        continue
    }

    $LASTEXITCODE = 0
    $code = 1
    try {
        # Capture every stream (Write-Output, Write-Host, warnings, errors)
        # merged into the submission's capture file.
        & $path *>&1 | Out-File -LiteralPath $outFile -Encoding utf8
        $code = $LASTEXITCODE
        if ($null -eq $code) { $code = 0 }
    } catch {
        try {
            Add-Content -LiteralPath $outFile -Value ('[[HOK_SCRIPT_ERROR]] ' + $_.Exception.Message) -Encoding utf8
        } catch {
            # capture file may be absent if even creating it failed
        }
        $code = 1
    }

    [Console]::Out.WriteLine('DONE_' + $token + ':' + $code)
    [Console]::Out.Flush()
}
