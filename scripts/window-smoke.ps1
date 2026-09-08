$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
$taskExe = Join-Path $taskRoot 'src-tauri/target/debug/locallm.exe'
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9223'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class LocalLMWindowTest {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr window, int x, int y, int width, int height, bool repaint);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr window);
}
'@
function Wait-Condition([scriptblock]$Check, [string]$Failure) {
  $deadline = [DateTime]::UtcNow.AddSeconds(15)
  do { if (& $Check) { return }; Start-Sleep -Milliseconds 100 } while ([DateTime]::UtcNow -lt $deadline)
  throw $Failure
}
function Start-LocalLM {
  $process = Start-Process -FilePath $taskExe -WorkingDirectory $taskRoot -WindowStyle Hidden -PassThru
  Wait-Condition { $process.Refresh(); !$process.HasExited -and $process.MainWindowHandle -ne 0 } 'No main window appeared'
  return $process
}
function Close-LocalLM($process) {
  $process.Refresh()
  if ($process.Path -ne $taskExe) { throw 'Unexpected executable path' }
  if (!$process.CloseMainWindow() -or !$process.WaitForExit(15000)) { throw 'LocalLM did not close cleanly' }
}
function Get-Rect($process) {
  $rect = New-Object LocalLMWindowTest+Rect
  if (![LocalLMWindowTest]::GetWindowRect($process.MainWindowHandle, [ref]$rect)) { throw 'Could not read window bounds' }
  return $rect
}
if (@(Get-Process locallm -ErrorAction SilentlyContinue | Where-Object Path -eq $taskExe).Count) { throw 'Close the test application before running this smoke test' }
$process = Start-LocalLM
$original = Get-Rect $process
$originalMaximized = [LocalLMWindowTest]::IsZoomed($process.MainWindowHandle)
try {
  $null = [LocalLMWindowTest]::ShowWindow($process.MainWindowHandle, 9)
  if (![LocalLMWindowTest]::MoveWindow($process.MainWindowHandle, 100, 80, 1050, 740, $true)) { throw 'MoveWindow failed' }
  Wait-Condition { $rect = Get-Rect $process; $rect.Left -eq 100 -and $rect.Top -eq 80 -and ($rect.Right-$rect.Left) -eq 1050 } 'Test bounds not applied'
  $expected = Get-Rect $process
  Close-LocalLM $process
  $process = Start-LocalLM
  Wait-Condition { $actual=Get-Rect $process; $actual.Left -eq $expected.Left -and $actual.Top -eq $expected.Top -and $actual.Right -eq $expected.Right -and $actual.Bottom -eq $expected.Bottom } 'Window bounds were not restored'
  $null = [LocalLMWindowTest]::ShowWindow($process.MainWindowHandle, 6)
  Wait-Condition { [LocalLMWindowTest]::IsIconic($process.MainWindowHandle) } 'Window did not minimize'
  $second = Start-Process -FilePath $taskExe -WorkingDirectory $taskRoot -WindowStyle Hidden -PassThru
  if (!$second.WaitForExit(15000)) { throw 'Second instance did not exit' }
  Wait-Condition { ![LocalLMWindowTest]::IsIconic($process.MainWindowHandle) } 'Second launch did not restore the existing window'
  $running = @(Get-Process locallm -ErrorAction SilentlyContinue | Where-Object Path -eq $taskExe)
  if ($running.Count -ne 1 -or $running[0].Id -ne $process.Id) { throw 'Second launch replaced or duplicated the original process' }
  $null = [LocalLMWindowTest]::ShowWindow($process.MainWindowHandle, 3)
  Wait-Condition { [LocalLMWindowTest]::IsZoomed($process.MainWindowHandle) } 'Window did not maximize'
  Close-LocalLM $process
  $process = Start-LocalLM
  Wait-Condition { [LocalLMWindowTest]::IsZoomed($process.MainWindowHandle) } 'Maximized state was not restored'
  $report = @{ testedAt=[DateTime]::UtcNow.ToString('o'); boundsRestored=$true; maximizedRestored=$true; secondLaunchReusesProcess=$true; minimizedWindowRestored=$true }
  $report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskRoot 'test-results/window-smoke.json')
  $report | ConvertTo-Json
} finally {
  if (!$process.HasExited) {
    $null = [LocalLMWindowTest]::ShowWindow($process.MainWindowHandle, 9)
    $null = [LocalLMWindowTest]::MoveWindow($process.MainWindowHandle, $original.Left, $original.Top, ($original.Right-$original.Left), ($original.Bottom-$original.Top), $true)
    if ($originalMaximized) { $null = [LocalLMWindowTest]::ShowWindow($process.MainWindowHandle, 3) }
  }
}
