$cases = [ordered]@{
    'plain'              = '{"a":1}'
    'trailing-comma-obj' = '{"a":1,}'
    'line-comment'       = '{"a":1 // c}'
    'line-comment-nl'    = "{`"a`":1 // c`n}"
    'block-comment'      = '{"a":1 /* c */}'
    'single-quote-key'   = "{'a':1}"
    'bare-key'           = '{a:1}'
    'leading-zero'       = '{"a":01}'
    'trailing-comma-arr' = '[1,2,]'
    'dup-key'            = '{"a":1,"a":2}'
    'garbage'            = '{ not json'
}
Write-Output ("HOST {0} {1}" -f $PSVersionTable.PSVersion, $PSVersionTable.PSEdition)
foreach ($k in $cases.Keys) {
    try {
        $null = $cases[$k] | ConvertFrom-Json -ErrorAction Stop
        Write-Output ("{0}`tOK" -f $k)
    }
    catch {
        Write-Output ("{0}`tFAIL" -f $k)
    }
}
