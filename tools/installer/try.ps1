# Tries the Windows setup program: installs Zeno without showing a
# window, checks that it is there and on the PATH and that it runs,
# uninstalls it, and checks that it is gone.
#
#     pwsh tools\installer\try.ps1 zeno-0.2.0-windows-setup.exe
#
# The release workflow runs this before a setup program is released.

param([Parameter(Mandatory = $true)][string]$Setup)
$ErrorActionPreference = 'Stop'

$setupFile = (Resolve-Path $Setup).Path
$app = Join-Path $env:LOCALAPPDATA 'Programs\Zeno'
$quiet = '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'

# The PATH of this user, as the registry has it.
function UserPath { [Environment]::GetEnvironmentVariable('Path', 'User') }

$log = Join-Path (Get-Location) 'setup.log'
$run = Start-Process -FilePath $setupFile -ArgumentList ($quiet + "/LOG=`"$log`"") -Wait -PassThru
if ($run.ExitCode -ne 0) {
    if (Test-Path $log) { Get-Content $log -Tail 30 }
    throw "the setup program ended with $($run.ExitCode)"
}

$zeno = Join-Path $app 'zeno.exe'
if (-not (Test-Path $zeno)) { throw "there is no $zeno" }
$said = & $zeno --version
Write-Output "installed: $said"
if ($said -notmatch '^zeno \d+\.\d+\.\d+$') { throw "zeno --version said: $said" }
if (";$(UserPath);" -notlike "*;$app;*") { throw "the PATH does not have $app in it: $(UserPath)" }
Write-Output "the PATH has $app"

foreach ($thing in 'zeno.wasm', 'README.md', 'docs\web.md', 'examples\hello.zeno', 'editors', 'unins000.exe') {
    if (-not (Test-Path (Join-Path $app $thing))) { throw "the installed folder has no $thing" }
}
$hello = & $zeno run (Join-Path $app 'examples\hello.zeno')
if ($hello -ne 'Hello, Zeno!') { throw "hello.zeno printed: $hello" }

# A page builds: `zeno.wasm` is found beside `zeno.exe`.
$work = Join-Path $env:TEMP "zeno-try-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
Push-Location $work
& $zeno build --web (Join-Path $app 'examples\todo.zeno')
if (-not (Test-Path 'todo.web.html')) { throw '`zeno build --web` wrote no page' }
Pop-Location
Remove-Item -Recurse -Force $work
Write-Output 'it runs a program and builds a page'

# Installed a second time, it is still in the PATH once.
$run = Start-Process -FilePath $setupFile -ArgumentList $quiet -Wait -PassThru
if ($run.ExitCode -ne 0) { throw "installing again ended with $($run.ExitCode)" }
$times = ((UserPath) -split ';' | Where-Object { $_ -eq $app }).Count
if ($times -ne 1) { throw "the PATH has $app in it $times times" }

$run = Start-Process -FilePath (Join-Path $app 'unins000.exe') -ArgumentList $quiet -Wait -PassThru
if ($run.ExitCode -ne 0) { throw "the uninstaller ended with $($run.ExitCode)" }
# The uninstaller finishes its work in a copy of itself.
foreach ($wait in 1..20) {
    if (-not (Test-Path $zeno)) { break }
    Start-Sleep -Milliseconds 500
}
if (Test-Path $zeno) { throw "after uninstalling, $zeno is still there" }
if (";$(UserPath);" -like "*;$app;*") { throw "after uninstalling, the PATH still has $app in it" }
Write-Output 'uninstalled: the folder and the PATH are as they were'
