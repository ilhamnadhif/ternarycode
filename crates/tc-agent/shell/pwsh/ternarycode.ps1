# TernaryCode shell integration for PowerShell 7 and Windows PowerShell 5.1: OSC 133 prompt marks
# (A prompt start, B prompt end, C command start, D;status command end) and the working
# directory (OSC 7). Passed with -EncodedCommand when a pane starts, after the user's profile.
if ($Global:__TernaryCode) { return }
$Global:__TernaryCode = @{ Running = $false; Wrapped = $false; Esc = [char]27; Bel = [char]7; Prompt = $function:prompt; ReadLine = $null }

function Global:__TernaryCodeWrapReadLine {
    $t = $Global:__TernaryCode
    # Without PSReadLine (not loaded yet) there is nothing to wrap: tried again at the next prompt.
    $original = Get-Command PSConsoleHostReadLine -CommandType Function -ErrorAction Ignore
    if (-not $original) { return }
    $t.Wrapped = $true
    $t.ReadLine = $original.ScriptBlock
    function Global:PSConsoleHostReadLine {
        $t = $Global:__TernaryCode
        $line = & $t.ReadLine
        # An empty line runs nothing: no C (and so no D), as in the other shells.
        if (-not [string]::IsNullOrWhiteSpace($line)) {
            $t.Running = $true
            [Console]::Write("$($t.Esc)]133;C$($t.Bel)")
        }
        $line
    }
}

function Global:prompt {
    $ok = $?
    $code = $Global:LASTEXITCODE
    $t = $Global:__TernaryCode
    $out = ''
    if ($t.Running) {
        $status = 0
        if (-not $ok) {
            if ($code -is [int] -and $code -ne 0) { $status = $code } else { $status = 1 }
        }
        $out += "$($t.Esc)]133;D;$status$($t.Bel)"
        $t.Running = $false
    }
    if (-not $t.Wrapped) { __TernaryCodeWrapReadLine }
    $loc = $ExecutionContext.SessionState.Path.CurrentLocation
    if ($loc.Provider.Name -eq 'FileSystem') {
        try { $out += "$($t.Esc)]7;$(([System.Uri]::new($loc.ProviderPath)).AbsoluteUri)$($t.Bel)" } catch {}
    }
    $out += "$($t.Esc)]133;A$($t.Bel)"
    # The user's prompt sees the command's status.
    $Global:LASTEXITCODE = $code
    if (-not $ok) { Write-Error 'failed' -ErrorAction Ignore }
    if ($t.Prompt) { $text = & $t.Prompt } else { $text = "PS $($loc.Path)$('>' * ($NestedPromptLevel + 1)) " }
    $Global:LASTEXITCODE = $code
    "$out$(-join $text)$($t.Esc)]133;B$($t.Bel)"
}
