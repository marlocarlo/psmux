param()

function Update-PmuxPaneTitle {
    try {
        $cwdLeaf = Split-Path -Leaf (Get-Location)
        $lastCmd = (Get-History -Count 1 | Select-Object -ExpandProperty CommandLine)
        $title = if ($lastCmd) { "$cwdLeaf: $lastCmd" } else { $cwdLeaf }
        pmux set-pane-title $title | Out-Null
    } catch {
        # Ignore errors if pmux or session is not running
    }
}

# Usage:
# 1) Add to your $PROFILE (Microsoft.PowerShell_profile.ps1):
#    . "$PSScriptRoot\pmux-title.ps1"
#    function Prompt {
#        Update-PmuxPaneTitle
#        "PS " + (Get-Location) + "> "
#    }
# 2) Ensure a running pmux session server and, if needed, set $env:PMUX_TARGET_SESSION.