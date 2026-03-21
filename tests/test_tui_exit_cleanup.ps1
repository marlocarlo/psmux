<#
.SYNOPSIS
  Test terminal state cleanup after TUI app exit inside psmux.
  
  Verifies that after a TUI app (pstop, opencode, claude, fake-crash TUI)
  exits, the psmux terminal is fully restored:
  - No garbled mouse escape sequences in the output
  - No TUI content remnants from alternate-screen apps
  - Shell prompt is visible and responsive
  - Typing produces correct output
  - Arrow-key cursor navigation works (visual matches actual)
  
  Root cause being tested: the Ctrl+C handler must NOT prematurely exit the
  alternate screen -- it must let the TUI app flush its own cleanup sequences
  via the reader thread.  Premature exit causes the TUI's final output to
  corrupt the primary grid.
#>
$ErrorActionPreference = "Continue"
$results = @()

function Add-Result($name, $pass, $detail="") {
    $script:results += [PSCustomObject]@{ Test=$name; Result=if($pass){"PASS"}else{"FAIL"}; Detail=$detail }
    $mark = if($pass) { "[PASS]" } else { "[FAIL]" }
    Write-Host "  $mark $name$(if($detail){' '+$detail}else{''})"
}

# Create a fake TUI that enables all terminal modes and exits cleanly
$fakeTuiClean = @'
$esc = [char]27
Write-Host -NoNewline "$esc[?1049h"
Write-Host -NoNewline "$esc[?1003h$esc[?1006h"
Write-Host -NoNewline "$esc[?1h"
Write-Host -NoNewline "$esc[?2004h"
Write-Host -NoNewline "$esc[2 q"
Write-Host -NoNewline "$esc[1;1H$esc[44m$esc[37m=== FAKE TUI APP ===$esc[0m"
Write-Host -NoNewline "$esc[2;1H$esc[42mProcess list here...$esc[0m"
Write-Host -NoNewline "$esc[3;1H$esc[41mCPU: 100%$esc[0m"
Start-Sleep -Seconds 2
Write-Host -NoNewline "$esc[?1003l$esc[?1006l"
Write-Host -NoNewline "$esc[?1l"
Write-Host -NoNewline "$esc[?2004l"
Write-Host -NoNewline "$esc[0m$esc[?25h"
Write-Host -NoNewline "$esc[?1049l"
'@

# Create a fake TUI that exits WITHOUT cleanup (simulates crash)
$fakeTuiCrash = @'
$esc = [char]27
Write-Host -NoNewline "$esc[?1049h"
Write-Host -NoNewline "$esc[?1003h$esc[?1006h"
Write-Host -NoNewline "$esc[?1h"
Write-Host -NoNewline "$esc[?2004h"
Write-Host -NoNewline "$esc[2 q"
Write-Host -NoNewline "$esc[1;1H$esc[44m$esc[37m=== CRASH TUI ===$esc[0m"
Write-Host -NoNewline "$esc[2;1H$esc[41mAbout to crash...$esc[0m"
Start-Sleep -Seconds 2
'@

$cleanScript = "$env:TEMP\fake_tui_clean.ps1"
$crashScript = "$env:TEMP\fake_tui_crash.ps1"
Set-Content -Path $cleanScript -Value $fakeTuiClean -Encoding UTF8
Set-Content -Path $crashScript -Value $fakeTuiCrash -Encoding UTF8

Write-Host "=== TUI Exit Cleanup Test ==="
Write-Host ""

psmux kill-server 2>$null
Start-Sleep -Seconds 1

# =====================================================================
# TEST GROUP 1: Clean TUI exit (sends RMCUP + disables modes)
# =====================================================================
Write-Host "--- Group 1: Clean TUI exit ---"
psmux new-session -d -s tui_clean -x 120 -y 30 2>$null
Start-Sleep -Seconds 3

psmux send-keys -t tui_clean "pwsh -NoProfile -File `"$cleanScript`"" Enter
Start-Sleep -Seconds 5

$cap = psmux capture-pane -t tui_clean -p 2>&1 | Out-String
$hasEscGarbage = $cap -match '\[[\d;]+[Mm]' -and $cap -match '555|1003|1006'
Add-Result "Clean exit: no mouse escape garbage" (-not $hasEscGarbage)

$hasTuiContent = $cap -match 'FAKE TUI APP' -or $cap -match 'Process list here' -or $cap -match 'CPU: 100%'
Add-Result "Clean exit: no TUI content remnants" (-not $hasTuiContent)

$hasPrompt = $cap -match 'PS [A-Z]:\\'
Add-Result "Clean exit: shell prompt visible" $hasPrompt

psmux send-keys -t tui_clean "echo cursor_test_ok" Enter
Start-Sleep -Seconds 1
$cap2 = psmux capture-pane -t tui_clean -p 2>&1 | Out-String
Add-Result "Clean exit: typing works" ($cap2 -match 'cursor_test_ok')

psmux send-keys -t tui_clean "echo arrow_ABC" ""
Start-Sleep -Milliseconds 500
psmux send-keys -t tui_clean Left Left Left ""
Start-Sleep -Milliseconds 500
psmux send-keys -t tui_clean "X" ""
Start-Sleep -Milliseconds 500
psmux send-keys -t tui_clean Enter
Start-Sleep -Seconds 1
$cap3 = psmux capture-pane -t tui_clean -p 2>&1 | Out-String
Add-Result "Clean exit: arrow keys work (cursor in sync)" ($cap3 -match 'arrow_XABC')

psmux kill-session -t tui_clean 2>$null
Start-Sleep -Seconds 1

# =====================================================================
# TEST GROUP 2: TUI crash (no cleanup) + Ctrl+C recovery
# =====================================================================
Write-Host "`n--- Group 2: TUI crash (no cleanup) ---"
psmux new-session -d -s tui_crash -x 120 -y 30 2>$null
Start-Sleep -Seconds 3

psmux send-keys -t tui_crash "pwsh -NoProfile -File `"$crashScript`"" Enter
Start-Sleep -Seconds 5

psmux send-keys -t tui_crash C-c
Start-Sleep -Seconds 3

$cap4 = psmux capture-pane -t tui_crash -p 2>&1 | Out-String
$hasEscGarbage2 = $cap4 -match '\[\d{2,};[\d;]+[Mm]'
Add-Result "Crash exit: no mouse escape garbage" (-not $hasEscGarbage2)

$hasPrompt2 = $cap4 -match 'PS [A-Z]:\\'
Add-Result "Crash exit: prompt visible after Ctrl+C" $hasPrompt2

psmux send-keys -t tui_crash "echo crash_test_ok" Enter
Start-Sleep -Seconds 1
$cap5 = psmux capture-pane -t tui_crash -p 2>&1 | Out-String
Add-Result "Crash exit: typing works" ($cap5 -match 'crash_test_ok')

psmux send-keys -t tui_crash "echo crash_DEF" ""
Start-Sleep -Milliseconds 500
psmux send-keys -t tui_crash Left Left Left ""
Start-Sleep -Milliseconds 500
psmux send-keys -t tui_crash "Y" ""
Start-Sleep -Milliseconds 500
psmux send-keys -t tui_crash Enter
Start-Sleep -Seconds 1
$cap6 = psmux capture-pane -t tui_crash -p 2>&1 | Out-String
Add-Result "Crash exit: arrow keys work" ($cap6 -match 'crash_YDEF')

# On ConPTY (Windows), a crashed TUI that never sent RMCUP will leave alt-screen
# content visible because ConPTY never receives the restore sequence.  This is
# expected behaviour (same as tmux).  Verify the shell IS responsive instead.
$shellResponsive = $cap5 -match 'crash_test_ok' -and ($cap4 -match 'PS [A-Z]:\\')
Add-Result "Crash exit: shell responsive despite TUI remnants" $shellResponsive

psmux kill-session -t tui_crash 2>$null
Start-Sleep -Seconds 1

# =====================================================================
# TEST GROUP 3: Real pstop.exe test (clean 'q' exit)
# On Windows, ConPTY sends Ctrl+C to all console processes, killing the
# shell alongside pstop.  Use pstop's 'q' key for a clean exit that sends
# RMCUP.  The Ctrl+C + crash behaviour is covered by Group 2 (fake TUI)
# and Group 3b (force-kill).
# =====================================================================
$pstopPath = Get-Command pstop.exe -ErrorAction SilentlyContinue
if ($pstopPath) {
    Write-Host "`n--- Group 3: pstop.exe (clean 'q' exit) ---"
    psmux new-session -d -s tui_pstop -x 120 -y 30 2>$null
    Start-Sleep -Seconds 3
    
    psmux send-keys -t tui_pstop "pstop.exe" Enter
    Start-Sleep -Seconds 4
    
    psmux send-keys -t tui_pstop "q"
    Start-Sleep -Seconds 6
    
    $capP = psmux capture-pane -t tui_pstop -p 2>&1 | Out-String
    
    $hasPstopRemnants = $capP -match 'CPU%.*MEM%|PID\s+PPID|Tasks:.*thr.*running'
    Add-Result "pstop clean exit: no TUI content on primary grid" (-not $hasPstopRemnants)
    
    $hasPstopGarbage = $capP -match '\[\d{2,};[\d;]+[Mm]'
    Add-Result "pstop clean exit: no garbled mouse sequences" (-not $hasPstopGarbage)
    
    $hasPstopPrompt = $capP -match 'PS [A-Z]:\\'
    Add-Result "pstop clean exit: shell prompt visible" $hasPstopPrompt
    
    psmux send-keys -t tui_pstop "echo pstop_test_ok" Enter
    Start-Sleep -Seconds 1
    $capP2 = psmux capture-pane -t tui_pstop -p 2>&1 | Out-String
    Add-Result "pstop clean exit: typing works" ($capP2 -match 'pstop_test_ok')
    
    psmux send-keys -t tui_pstop "echo pstop_GHI" ""
    Start-Sleep -Milliseconds 500
    psmux send-keys -t tui_pstop Left Left Left ""
    Start-Sleep -Milliseconds 500
    psmux send-keys -t tui_pstop "Z" ""
    Start-Sleep -Milliseconds 500
    psmux send-keys -t tui_pstop Enter
    Start-Sleep -Seconds 1
    $capP3 = psmux capture-pane -t tui_pstop -p 2>&1 | Out-String
    Add-Result "pstop clean exit: arrow keys work (cursor in sync)" ($capP3 -match 'pstop_ZGHI')
    
    psmux kill-session -t tui_pstop 2>$null
    Start-Sleep -Seconds 1

    # --- pstop crash case: Ctrl+C then force-kill (no RMCUP) ---
    Write-Host "`n--- Group 3b: pstop force-kill crash (no RMCUP) ---"
    psmux new-session -d -s tui_pstop_fk -x 120 -y 30 2>$null
    Start-Sleep -Seconds 4
    
    psmux send-keys -t tui_pstop_fk "pstop.exe" Enter
    Start-Sleep -Seconds 5
    
    # Ctrl+C (sets ctrl_c_at) then immediately force-kill
    psmux send-keys -t tui_pstop_fk C-c
    Start-Sleep -Milliseconds 200
    Get-Process -Name "pstop" -ErrorAction SilentlyContinue | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    
    # Wait for 2s timeout + buffer
    Start-Sleep -Seconds 8
    
    $capFK = psmux capture-pane -t tui_pstop_fk -p 2>&1 | Out-String
    # On ConPTY (Windows), a force-killed TUI that never sent RMCUP will leave
    # alt-screen content visible.  Verify the shell IS responsive instead.
    $hasFK_Prompt = $capFK -match 'PS [A-Z]:\\'
    Add-Result "pstop force-kill: prompt visible" $hasFK_Prompt
    
    psmux send-keys -t tui_pstop_fk "echo fk_test_ok" Enter
    Start-Sleep -Seconds 1
    $capFK2 = psmux capture-pane -t tui_pstop_fk -p 2>&1 | Out-String
    Add-Result "pstop force-kill: typing works" ($capFK2 -match 'fk_test_ok')
    
    psmux kill-session -t tui_pstop_fk 2>$null
    Start-Sleep -Seconds 1
} else {
    Write-Host "`n--- Group 3: pstop.exe not found, skipping ---"
}

# =====================================================================
# TEST GROUP 4: Real opencode test (Ctrl+C exit)
# =====================================================================
$opencodePath = Get-Command opencode -ErrorAction SilentlyContinue
if ($opencodePath) {
    Write-Host "`n--- Group 4: opencode (Ctrl+C exit) ---"
    psmux new-session -d -s tui_oc -x 120 -y 30 2>$null
    Start-Sleep -Seconds 3
    
    psmux send-keys -t tui_oc "cd c:\cctest && opencode" Enter
    Start-Sleep -Seconds 8
    
    psmux send-keys -t tui_oc C-c
    Start-Sleep -Seconds 4
    
    $capOC = psmux capture-pane -t tui_oc -p 2>&1 | Out-String
    
    $hasOcGarbage = $capOC -match '\[\d{2,};[\d;]+[Mm]'
    Add-Result "opencode Ctrl+C: no garbled mouse sequences" (-not $hasOcGarbage)
    
    $hasOcPrompt = $capOC -match 'PS [A-Z]:\\'
    Add-Result "opencode Ctrl+C: shell prompt visible" $hasOcPrompt
    
    psmux send-keys -t tui_oc "echo oc_test_ok" Enter
    Start-Sleep -Seconds 1
    $capOC2 = psmux capture-pane -t tui_oc -p 2>&1 | Out-String
    Add-Result "opencode Ctrl+C: typing works" ($capOC2 -match 'oc_test_ok')
    
    psmux send-keys -t tui_oc "echo oc_RST" ""
    Start-Sleep -Milliseconds 500
    psmux send-keys -t tui_oc Left Left Left ""
    Start-Sleep -Milliseconds 500
    psmux send-keys -t tui_oc "V" ""
    Start-Sleep -Milliseconds 500
    psmux send-keys -t tui_oc Enter
    Start-Sleep -Seconds 1
    $capOC3 = psmux capture-pane -t tui_oc -p 2>&1 | Out-String
    Add-Result "opencode Ctrl+C: arrow keys work" ($capOC3 -match 'oc_VRST')
    
    psmux kill-session -t tui_oc 2>$null
    Start-Sleep -Seconds 1
} else {
    Write-Host "`n--- Group 4: opencode not found, skipping ---"
}

# =====================================================================
# TEST GROUP 5: Multiple TUI launches in same pane
# =====================================================================
Write-Host "`n--- Group 5: Multiple TUI launches ---"
psmux new-session -d -s tui_multi -x 120 -y 30 2>$null
Start-Sleep -Seconds 3

for ($i = 1; $i -le 3; $i++) {
    psmux send-keys -t tui_multi "pwsh -NoProfile -File `"$cleanScript`"" Enter
    Start-Sleep -Seconds 5
}

psmux send-keys -t tui_multi "echo multi_test_ok" Enter
Start-Sleep -Seconds 1
$capM = psmux capture-pane -t tui_multi -p 2>&1 | Out-String
Add-Result "Multi TUI: terminal works after 3 launches" ($capM -match 'multi_test_ok')

$hasMultiGarbage = $capM -match '\[\d{2,};[\d;]+[Mm]'
Add-Result "Multi TUI: no escape garbage" (-not $hasMultiGarbage)

psmux send-keys -t tui_multi "echo multi_JKL" ""
Start-Sleep -Milliseconds 500
psmux send-keys -t tui_multi Left Left Left ""
Start-Sleep -Milliseconds 500
psmux send-keys -t tui_multi "W" ""
Start-Sleep -Milliseconds 500
psmux send-keys -t tui_multi Enter
Start-Sleep -Seconds 1
$capM2 = psmux capture-pane -t tui_multi -p 2>&1 | Out-String
Add-Result "Multi TUI: arrow keys work after 3 launches" ($capM2 -match 'multi_WJKL')

psmux kill-session -t tui_multi 2>$null
Start-Sleep -Seconds 1

# =====================================================================
# TEST GROUP 6: pstop then opencode back-to-back in same pane
# =====================================================================
if ($pstopPath -and $opencodePath) {
    Write-Host "`n--- Group 6: pstop then opencode back-to-back ---"
    psmux new-session -d -s tui_combo -x 120 -y 30 2>$null
    Start-Sleep -Seconds 3
    
    psmux send-keys -t tui_combo "pstop.exe" Enter
    Start-Sleep -Seconds 4
    psmux send-keys -t tui_combo C-c
    Start-Sleep -Seconds 4
    
    psmux send-keys -t tui_combo "cd c:\cctest && opencode" Enter
    Start-Sleep -Seconds 8
    psmux send-keys -t tui_combo C-c
    Start-Sleep -Seconds 4
    
    psmux send-keys -t tui_combo "echo combo_test_ok" Enter
    Start-Sleep -Seconds 1
    $capC = psmux capture-pane -t tui_combo -p 2>&1 | Out-String
    Add-Result "Combo test: typing works" ($capC -match 'combo_test_ok')
    
    $hasCGarbage = $capC -match '\[\d{2,};[\d;]+[Mm]'
    Add-Result "Combo test: no garbled text" (-not $hasCGarbage)
    
    psmux send-keys -t tui_combo "echo combo_ABC" ""
    Start-Sleep -Milliseconds 500
    psmux send-keys -t tui_combo Left Left Left ""
    Start-Sleep -Milliseconds 500
    psmux send-keys -t tui_combo "X" ""
    Start-Sleep -Milliseconds 500
    psmux send-keys -t tui_combo Enter
    Start-Sleep -Seconds 1
    $capC2 = psmux capture-pane -t tui_combo -p 2>&1 | Out-String
    Add-Result "Combo test: arrow keys work" ($capC2 -match 'combo_XABC')
    
    psmux kill-session -t tui_combo 2>$null
    Start-Sleep -Seconds 1
} else {
    Write-Host "`n--- Group 6: Skipped (requires both pstop + opencode) ---"
}

# =====================================================================
# TEST GROUP 7: Screen cleanliness
# =====================================================================
Write-Host "`n--- Group 7: Screen cleanliness ---"
psmux new-session -d -s tui_clean_chk -x 120 -y 30 2>$null
Start-Sleep -Seconds 3

psmux send-keys -t tui_clean_chk "pwsh -NoProfile -File `"$cleanScript`"" Enter
Start-Sleep -Seconds 5

for ($i = 1; $i -le 5; $i++) {
    psmux send-keys -t tui_clean_chk "echo line_$i" Enter
    Start-Sleep -Milliseconds 300
}
Start-Sleep -Seconds 1

$capClean = psmux capture-pane -t tui_clean_chk -p 2>&1 | Out-String
$lines = ($capClean -split "`n") | Where-Object { $_.Trim().Length -gt 0 }
Add-Result "Screen clean: output lines visible" ($lines.Count -ge 5) "($($lines.Count) non-empty lines)"

$allLinesPresent = $true
for ($i = 1; $i -le 5; $i++) {
    if ($capClean -notmatch "line_$i") { $allLinesPresent = $false; break }
}
Add-Result "Screen clean: all typed lines present" $allLinesPresent

psmux kill-session -t tui_clean_chk 2>$null

# =====================================================================
# TEST GROUP 8: TUI exit in split panes
# =====================================================================
Write-Host "`n--- Group 8: TUI exit in split panes ---"
if ($pstopPath) {
    # Vertical split
    psmux new-session -d -s tui_split -x 120 -y 30 2>$null
    Start-Sleep -Seconds 2
    psmux split-window -t tui_split -v
    Start-Sleep -Seconds 2
    psmux send-keys -t tui_split "pstop.exe" Enter
    Start-Sleep -Seconds 4
    psmux send-keys -t tui_split "q"
    Start-Sleep -Seconds 6
    $capSV = psmux capture-pane -t tui_split -p 2>&1 | Out-String
    $hasSV = $capSV -match 'CPU%|F1Help|PID\s+PPID'
    Add-Result "Split-V: no pstop remnants" (-not $hasSV)
    psmux send-keys -t tui_split "echo split_v_ok" Enter
    Start-Sleep -Seconds 1
    $capSV2 = psmux capture-pane -t tui_split -p 2>&1 | Out-String
    Add-Result "Split-V: typing works" ($capSV2 -match 'split_v_ok')
    psmux kill-session -t tui_split 2>$null
    Start-Sleep -Seconds 1

    # Horizontal split
    psmux new-session -d -s tui_splith -x 120 -y 30 2>$null
    Start-Sleep -Seconds 2
    psmux split-window -t tui_splith -h
    Start-Sleep -Seconds 2
    psmux send-keys -t tui_splith "pstop.exe" Enter
    Start-Sleep -Seconds 4
    psmux send-keys -t tui_splith "q"
    Start-Sleep -Seconds 6
    $capSH = psmux capture-pane -t tui_splith -p 2>&1 | Out-String
    $hasSH = $capSH -match 'CPU%|F1Help|PID\s+PPID'
    Add-Result "Split-H: no pstop remnants" (-not $hasSH)
    psmux send-keys -t tui_splith "echo split_h_ok" Enter
    Start-Sleep -Seconds 1
    $capSH2 = psmux capture-pane -t tui_splith -p 2>&1 | Out-String
    Add-Result "Split-H: typing works" ($capSH2 -match 'split_h_ok')
    psmux kill-session -t tui_splith 2>$null
    Start-Sleep -Seconds 1

    # Both panes in a split running pstop simultaneously
    psmux new-session -d -s tui_multi -x 120 -y 30 2>$null
    Start-Sleep -Seconds 2
    psmux split-window -t tui_multi -v
    Start-Sleep -Seconds 2
    psmux send-keys -t "tui_multi:0.1" "pstop.exe" Enter
    Start-Sleep -Seconds 2
    psmux select-pane -t "tui_multi:0.0"
    Start-Sleep -Milliseconds 500
    psmux send-keys -t "tui_multi:0.0" "pstop.exe" Enter
    Start-Sleep -Seconds 4
    psmux send-keys -t "tui_multi:0.0" "q"
    psmux send-keys -t "tui_multi:0.1" "q"
    Start-Sleep -Seconds 6
    $capM0 = psmux capture-pane -t "tui_multi:0.0" -p 2>&1 | Out-String
    $capM1 = psmux capture-pane -t "tui_multi:0.1" -p 2>&1 | Out-String
    $hm0 = $capM0 -match 'CPU%|F1Help|PID\s+PPID'
    $hm1 = $capM1 -match 'CPU%|F1Help|PID\s+PPID'
    Add-Result "Multi-split pane0: no pstop remnants" (-not $hm0)
    Add-Result "Multi-split pane1: no pstop remnants" (-not $hm1)
    psmux kill-session -t tui_multi 2>$null
    Start-Sleep -Seconds 1
} else {
    Write-Host "  [SKIP] pstop not found"
}

# =====================================================================
# TEST GROUP 9: TUI exit in new window (not initial session window)
# =====================================================================
Write-Host "`n--- Group 9: TUI exit in new window ---"
if ($pstopPath) {
    psmux new-session -d -s tui_newwin -x 120 -y 30 2>$null
    Start-Sleep -Seconds 2
    psmux new-window -t tui_newwin
    Start-Sleep -Milliseconds 500
    psmux send-keys -t tui_newwin "pstop.exe" Enter
    Start-Sleep -Seconds 3
    psmux send-keys -t tui_newwin "q"
    Start-Sleep -Seconds 6
    $capNW = psmux capture-pane -t tui_newwin -p 2>&1 | Out-String
    $hasNW = $capNW -match 'CPU%|F1Help|PID\s+PPID'
    Add-Result "New-window: no pstop remnants" (-not $hasNW)
    psmux send-keys -t tui_newwin "echo newwin_ok" Enter
    Start-Sleep -Seconds 1
    $capNW2 = psmux capture-pane -t tui_newwin -p 2>&1 | Out-String
    Add-Result "New-window: typing works" ($capNW2 -match 'newwin_ok')
    psmux kill-session -t tui_newwin 2>$null
    Start-Sleep -Seconds 1

    # New window with split inside
    psmux new-session -d -s tui_nwsplit -x 120 -y 30 2>$null
    Start-Sleep -Seconds 2
    psmux new-window -t tui_nwsplit
    Start-Sleep -Milliseconds 300
    psmux split-window -t tui_nwsplit -v
    Start-Sleep -Seconds 2
    psmux send-keys -t tui_nwsplit "pstop.exe" Enter
    Start-Sleep -Seconds 4
    psmux send-keys -t tui_nwsplit "q"
    Start-Sleep -Seconds 6
    $capNWS = psmux capture-pane -t tui_nwsplit -p 2>&1 | Out-String
    $hasNWS = $capNWS -match 'CPU%|F1Help|PID\s+PPID'
    Add-Result "New-win+split: no pstop remnants" (-not $hasNWS)
    psmux send-keys -t tui_nwsplit "echo nwsplit_ok" Enter
    Start-Sleep -Seconds 1
    $capNWS2 = psmux capture-pane -t tui_nwsplit -p 2>&1 | Out-String
    Add-Result "New-win+split: typing works" ($capNWS2 -match 'nwsplit_ok')
    psmux kill-session -t tui_nwsplit 2>$null
    Start-Sleep -Seconds 1
} else {
    Write-Host "  [SKIP] pstop not found"
}

# Cleanup
Remove-Item $cleanScript -Force -ErrorAction SilentlyContinue
Remove-Item $crashScript -Force -ErrorAction SilentlyContinue
psmux kill-server 2>$null

# --- Summary ---
Write-Host "`n=== RESULTS ==="
$results | Format-Table -AutoSize | Out-String | Write-Host
$pass = ($results | Where-Object { $_.Result -eq "PASS" }).Count
$fail = ($results | Where-Object { $_.Result -eq "FAIL" }).Count
Write-Host "Total: $($results.Count)  Pass: $pass  Fail: $fail"
if ($fail -gt 0) { exit 1 } else { exit 0 }
