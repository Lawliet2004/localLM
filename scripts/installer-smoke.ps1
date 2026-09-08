$ErrorActionPreference = 'Stop'
$taskRoot = Split-Path $PSScriptRoot -Parent
Set-Location -LiteralPath $taskRoot
$taskInstall = Join-Path $taskRoot '.local/installer-smoke'
$taskExe = Join-Path $taskInstall 'locallm.exe'
$taskInstaller = Join-Path $taskRoot 'src-tauri/target/release/bundle/nsis/LocalLM_0.1.0_x64-setup.exe'
$taskDb = Join-Path $env:APPDATA 'app.locallm.desktop/locallm.sqlite'
function Get-TestRegistration {
  Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*' -ErrorAction SilentlyContinue | Where-Object DisplayName -eq 'LocalLM'
}
if (Get-TestRegistration) { throw 'An existing LocalLM installation must not be overwritten by this test' }
if (Test-Path -LiteralPath $taskInstall) { throw 'Test installation directory already exists' }
if (Get-Process locallm -ErrorAction SilentlyContinue) { throw 'Close LocalLM before this installation test' }
$taskProcess = Start-Process -FilePath $taskInstaller -ArgumentList @('/S',("/D="+$taskInstall)) -WindowStyle Hidden -PassThru
if (!$taskProcess.WaitForExit(60000)) { throw 'Installer is still running' }
if ($taskProcess.ExitCode -ne 0 -or !(Test-Path -LiteralPath $taskExe)) { throw 'Installation failed' }
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9224'
$taskApp = Start-Process -FilePath $taskExe -WindowStyle Hidden -PassThru
$deadline = [DateTime]::UtcNow.AddSeconds(15)
do {
  try { $ready = Invoke-RestMethod http://127.0.0.1:9224/json/list -TimeoutSec 1 } catch { $ready = $null }
  if ($ready) { break }
  Start-Sleep -Milliseconds 100
} while ([DateTime]::UtcNow -lt $deadline)
try {
  node scripts/release-smoke.mjs
  if ($LASTEXITCODE -ne 0) { throw 'Installed application smoke failed' }
} finally {
  $taskApp.Refresh()
  if (!$taskApp.HasExited) {
    if ($taskApp.Path -ne $taskExe) { throw 'Unexpected application path' }
    $null = $taskApp.CloseMainWindow()
    if (!$taskApp.WaitForExit(15000)) { throw 'Installed app did not close' }
  }
}
$before = (Get-FileHash -LiteralPath $taskDb).Hash
$taskUninstall = Start-Process -FilePath (Join-Path $taskInstall 'uninstall.exe') -ArgumentList '/S' -WindowStyle Hidden -PassThru
if (!$taskUninstall.WaitForExit(30000)) { throw 'Uninstaller is still running' }
# NSIS hands off to a temporary uninstaller; its launcher may exit before registry cleanup.
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while ((Test-Path -LiteralPath $taskInstall) -or (Get-TestRegistration)) {
  if ([DateTime]::UtcNow -gt $deadline) { throw 'Uninstallation has not removed its files and registration' }
  Start-Sleep -Milliseconds 200
}
if ((Get-FileHash -LiteralPath $taskDb).Hash -ne $before) { throw 'Uninstallation changed the conversation database' }
$report = @{
  testedAt=[DateTime]::UtcNow.ToString('o'); installerBytes=(Get-Item -LiteralPath $taskInstaller).Length
  sha256=(Get-FileHash -LiteralPath $taskInstaller).Hash; signature=(Get-AuthenticodeSignature -LiteralPath $taskInstaller).Status.ToString()
  installExit=$taskProcess.ExitCode; uninstallLauncherExit=$taskUninstall.ExitCode
  appRemoved=$true; registrationRemoved=$true; databaseUnchanged=$true
}
$report | ConvertTo-Json | Set-Content -LiteralPath 'test-results/installer-smoke.json'
$report | ConvertTo-Json
