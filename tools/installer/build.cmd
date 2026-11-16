@echo off
rem Builds the Windows setup program from tools\installer\zeno.iss.
rem
rem     tools\installer\build.cmd 0.2.0 files
rem
rem The first word is the version, and the second the folder of files
rem that zeno.iss describes. The result is
rem zeno-<version>-windows-setup.exe, in the folder this is run in.
rem It needs Inno Setup 6, which GitHub's Windows machines have.

if "%~2"=="" (
    echo build.cmd: say the version and the folder of files, e.g. build.cmd 0.2.0 files
    exit /b 1
)
set "ISCC=%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe"
if not exist "%ISCC%" (
    echo build.cmd: Inno Setup 6 is not installed: there is no "%ISCC%"
    exit /b 1
)
"%ISCC%" /Qp "/DVersion=%~1" "/DFiles=%~f2" "/O%CD%" "%~dp0zeno.iss"
