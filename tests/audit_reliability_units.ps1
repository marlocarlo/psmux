#requires -Version 7.0
# Execute only reviewed unit modules; never pass an installed psmux executable.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$TestExecutable)
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$testExe = (Get-Item -LiteralPath $TestExecutable).FullName
if ([IO.Path]::GetFileName($testExe) -notmatch '^psmux-[0-9a-f]+\.exe$' -or
    [IO.Path]::GetFileName([IO.Path]::GetDirectoryName($testExe)) -ne 'deps') {
    throw 'Supply the Cargo-built psmux unit-test executable from a deps directory, never an installed CLI.'
}
$runDir = Join-Path $repoRoot ('target/audit/units-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,6))
New-Item -ItemType Directory -Path $runDir | Out-Null
$filters = @(
    'types::request_queue::tests::',
    'paths::registry_encoding_tests::',
    'registry::reliability_registry_tests::',
    'server::connection::bounded_wire_tests::',
    'server::connection::new_session_destination_tests::',
    'server::connection::session_command_routing_tests::',
    'input::tests_audit_input_errors::',
    'pane::io_queue::tests::',
    'pane::staging::tests::',
    'pane::pipe::tests::',
    'pane::tests_pane_writer_queue::',
    'pane::tests_pane_writer_transient_error::',
    'session::tests_session_id_alloc_race::',
    'server::panic_registration_tests::',
    'session::tests_reliability_discovery::',
    'session::tests::',
    'session::tests_issue448_orphan_reaper::',
    'session::tests_issue510_reaper_attribution::',
    'session::tests_issue509_namespace_instance::',
    'session::tests_picker_namespace_filter::',
    'session::tests_l_socket_tmux_precedence::',
    'platform::tests_issue599_data_root_mutex::',
    'server::tests_issue505_rename_session_guard::',
    'server::tests_issue574_rename_loop_guard::',
    'server::test_issue459_warm_single_instance::',
    'server::connection::tests_pane_border_indicator_control::',
    'server::connection::tests_refresh_client_flags::'
)
$results = [Collections.Generic.List[object]]::new()
foreach ($filter in $filters) {
    $tag = $filter.TrimEnd(':').Replace('::','-')
    $psi = [Diagnostics.ProcessStartInfo]::new($testExe)
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.ArgumentList.Add($filter)
    $psi.ArgumentList.Add('--test-threads=1')
    foreach ($key in @($psi.Environment.Keys)) {
        if ($key -match '^TMUX' -or $key -match '^PSMUX_') { $null = $psi.Environment.Remove($key) }
    }
    $psi.Environment['PSMUX_DATA_DIR'] = Join-Path $runDir 'isolated-data'
    $psi.Environment['PSMUX_NO_WARM'] = '1'
    $process = [Diagnostics.Process]::Start($psi)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $finished = $process.WaitForExit(45000)
    if (-not $finished) { $process.Kill(); $process.WaitForExit(3000) | Out-Null }
    $out = $stdout.GetAwaiter().GetResult()
    $err = $stderr.GetAwaiter().GetResult()
    [IO.File]::WriteAllText((Join-Path $runDir ($tag + '.stdout.txt')), $out)
    [IO.File]::WriteAllText((Join-Path $runDir ($tag + '.stderr.txt')), $err)
    $count = if ($out -match 'test result: ok\. (\d+) passed;') { [int]$Matches[1] } else { 0 }
    $result = [pscustomobject]@{ Filter=$filter; ExitCode=$process.ExitCode; Passed=$count; TimedOut=(-not $finished) }
    $results.Add($result)
    $process.Dispose()
    Write-Output ($result | ConvertTo-Json -Compress)
    $results | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $runDir 'results.json')
    if (-not $finished -or $result.ExitCode -ne 0 -or $count -eq 0) { throw "Unit filter failed: $filter. Evidence: $runDir" }
}
Write-Output "Unit evidence: $runDir"
Write-Output "Total passed: $(($results.Passed | Measure-Object -Sum).Sum)"
exit 0
