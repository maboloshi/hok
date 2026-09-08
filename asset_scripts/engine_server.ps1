# hok persistent PowerShell engine (server side).
#
# Kept alive by PsEngine (package/operations/psengine.rs) and driven over
# stdin. It reads one submission frame per line:
#
#     RUN <json>
#
# where <json> is a single-line object:
#     { "t": token, "p": script path, "env": {NAME: value, ...} }
#
# For each frame it resets the worker's process environment back to the
# spawn-time snapshot (so env_set from one submission cannot leak into the
# next — equivalent to Scoop spawning a fresh pwsh per script), then applies
# the frame's `env`, runs the script at `p`, and streams every line of its
# output (all streams, merged) to *this process's stdout* as it is produced.
# hok drains that stdout continuously and forwards each non-terminator line
# to its own consumer (the event bus), so a running script's output shows up
# live rather than only when the script exits. When the script finishes the
# engine writes a terminator line to stdout:
#
#     DONE_<token>:<exitcode>
#
# The exitcode is the script's LASTEXITCODE (0 when none). A thrown error
# emits a `[[HOK_SCRIPT_ERROR]] <message>` line (preserving the marker
# semantics the capture-file design used) and yields exitcode 1. QUIT on its
# own line stops the loop.
#
# Only one submission is in flight at a time (hok holds a single-submitter
# mutex), so every line on stdout between a RUN and the matching DONE_<token>
# belongs to that submission and hok need not multiplex. hok keeps reading
# stdout until it sees DONE_<token>, so a chatty script cannot fill the pipe.
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

# Snapshot the environment as seen at spawn. Each submission restores this
# before applying its own env, giving fresh-process semantics.
$baseEnv = @{}
foreach ($prop in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
    $baseEnv[$prop.Key] = [string]$prop.Value
}

# Reset the process env to the spawn-time snapshot: unset anything a previous
# submission added/changed and restore every snapshot key to its base value.
function Reset-Env {
    $current = [Environment]::GetEnvironmentVariables()
    foreach ($prop in $current.GetEnumerator()) {
        if (-not $baseEnv.ContainsKey($prop.Key)) {
            [Environment]::SetEnvironmentVariable([string]$prop.Key, $null)
        }
    }
    foreach ($entry in $baseEnv.GetEnumerator()) {
        [Environment]::SetEnvironmentVariable([string]$entry.Key, [string]$entry.Value)
    }
}

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

    Reset-Env
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
    $errorText = $null
    try {
        # Stream every stream (Write-Output, Write-Host, warnings, errors)
        # merged, line by line, to our stdout as the script produces it, so
        # hok can forward it live. Flush each line — a line left sitting in
        # the console buffer until the script ends would defeat streaming.
        & $path *>&1 | ForEach-Object {
            if ($null -ne $_) {
                [Console]::Out.WriteLine([string]$_)
                [Console]::Out.Flush()
            }
        }
        $code = $LASTEXITCODE
        if ($null -eq $code) { $code = 0 }
    } catch {
        $errorText = $_.Exception.Message
        $code = 1
    }

    if ($null -ne $errorText) {
        [Console]::Out.WriteLine('[[HOK_SCRIPT_ERROR]] ' + $errorText)
        [Console]::Out.Flush()
    }
    [Console]::Out.WriteLine('DONE_' + $token + ':' + $code)
    [Console]::Out.Flush()
}
