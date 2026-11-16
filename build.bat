@echo off
setlocal EnableExtensions
chcp 65001 >nul
title Spider - build
cd /d "%~dp0"

rem  Colours: the ESC character, then Spider's gradient (Venom theme).
for /F %%e in ('echo prompt $E^| cmd') do set "ESC=%%e"
set "R=%ESC%[0m"
set "B=%ESC%[1m"
set "C1=%ESC%[38;2;255;60;172m"
set "C2=%ESC%[38;2;213;80;207m"
set "C3=%ESC%[38;2;171;84;239m"
set "C4=%ESC%[38;2;137;95;255m"
set "C5=%ESC%[38;2;103;116;255m"
set "C6=%ESC%[38;2;43;134;255m"
set "MUTED=%ESC%[38;2;150;155;168m"
set "FAINT=%ESC%[38;2;88;92;104m"
set "GOOD=%ESC%[38;2;87;242;135m"
set "WARN=%ESC%[38;2;254;200;64m"
set "BAD=%ESC%[38;2;255;92;102m"

cls
echo.
echo   %B%%C1%S %C2%P %C3%I %C4%D %C5%E %C6%R%R%   %FAINT%·%R%  %MUTED%compilation%R%
echo   %C1%──────%C2%──────%C3%──────%C4%──────%C5%──────%C6%──────%R%
echo.

rem  1. Rust (cargo) is needed to build.
where cargo >nul 2>nul
if not errorlevel 1 goto build
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" (
  set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
  goto build
)
echo   %BAD%●%R%  Rust n'est pas installé : il faut %B%cargo%R% pour compiler Spider.
echo.
where winget >nul 2>nul
if errorlevel 1 goto norust
choice /C ON /N /M "   Installer Rust maintenant avec winget ? [O/N] "
if errorlevel 2 goto norust
echo.
winget install --id Rustlang.Rustup -e
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where cargo >nul 2>nul
if not errorlevel 1 goto build
echo.
echo   %WARN%●%R%  Rust vient d'être installé : ferme cette fenêtre et relance %B%build.bat%R%.
goto end

:norust
echo   %MUTED%   Installe Rust depuis https://rustup.rs, puis relance build.bat.%R%
goto end

rem  2. Build, then put Spider.exe beside this file.
:build
for /F "tokens=2" %%v in ('cargo --version') do set "CARGO_VERSION=%%v"
echo   %GOOD%●%R%  Rust %CARGO_VERSION% trouvé
echo   %FAINT%●%R%  %MUTED%Compilation en mode release (la première fois : une à deux minutes)…%R%
echo.
cargo build --release
if errorlevel 1 (
  echo.
  echo   %BAD%●%R%  La compilation a échoué : le message d'erreur est juste au-dessus.
  goto end
)
copy /Y "target\release\spider.exe" "Spider.exe" >nul 2>nul
if errorlevel 1 (
  echo.
  echo   %BAD%●%R%  Impossible d'écrire Spider.exe : il est sans doute ouvert. Ferme-le et relance.
  goto end
)
for %%s in ("Spider.exe") do set /a "SIZE=%%~zs / 1024"
echo.
echo   %GOOD%●%R%  %B%Spider.exe est prêt%R%  %FAINT%%SIZE% Ko · %CD%\Spider.exe%R%
echo.

rem  3. Spider opens right here, in this window.
choice /C ON /N /M "   Lancer Spider maintenant ? [O/N] "
if errorlevel 2 goto end
"%~dp0Spider.exe"
exit /b 0

:end
echo.
pause
exit /b 0
