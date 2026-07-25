# Build a double-clickable Windows app (no dev server).
# Outputs:
#   dist\3MF Profile Transplant.exe          (portable)
#   dist\3MF Profile Transplant-setup.exe    (installer, if NSIS build succeeds)
#   dist\README-for-users.txt

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$DesktopApp = Join-Path $Root "apps\desktop"
$Dist = Join-Path $Root "dist"
$ReleaseDir = Join-Path $Root "target\release"

Write-Host "==> Building frontend + release app (this can take several minutes)..." -ForegroundColor Cyan
Push-Location $DesktopApp
try {
  if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    throw "pnpm not found. Install Node.js and pnpm, then re-run."
  }
  pnpm install
  pnpm tauri build
} finally {
  Pop-Location
}

New-Item -ItemType Directory -Force -Path $Dist | Out-Null

$BuiltExe = Join-Path $ReleaseDir "desktop.exe"
if (-not (Test-Path $BuiltExe)) {
  throw "Release binary not found at $BuiltExe"
}

$PortableName = "3MF Profile Transplant.exe"
$PortablePath = Join-Path $Dist $PortableName
Copy-Item -Force $BuiltExe $PortablePath
Write-Host "==> Portable app: $PortablePath" -ForegroundColor Green

$Nsis = Get-ChildItem -Path (Join-Path $ReleaseDir "bundle\nsis") -Filter "*-setup.exe" -ErrorAction SilentlyContinue |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1
if ($Nsis) {
  $SetupPath = Join-Path $Dist "3MF Profile Transplant-setup.exe"
  Copy-Item -Force $Nsis.FullName $SetupPath
  Write-Host "==> Installer:     $SetupPath" -ForegroundColor Green
}

$Msi = Get-ChildItem -Path (Join-Path $ReleaseDir "bundle\msi") -Filter "*.msi" -ErrorAction SilentlyContinue |
  Sort-Object LastWriteTime -Descending |
  Select-Object -First 1
if ($Msi) {
  $MsiPath = Join-Path $Dist "3MF Profile Transplant.msi"
  Copy-Item -Force $Msi.FullName $MsiPath
  Write-Host "==> MSI:           $MsiPath" -ForegroundColor Green
}

$Readme = @"
3MF Profile Transplant — for end users
======================================

What to use
-----------
• Double-click:  3MF Profile Transplant.exe
  (portable — no install; leave it anywhere you like)

• Or run the installer once:
  3MF Profile Transplant-setup.exe
  then open the app from the Start menu like any other program.

Requirements (Windows)
----------------------
• Windows 10 or 11 (64-bit)
• Microsoft Edge WebView2 Runtime
  (already on most PCs; if the app won't open, install the free
   "Evergreen Bootstrapper" from Microsoft)

How to convert a file
---------------------
1. Open the app (double-click the .exe).
2. Choose your Bambu / MakerWorld project (.3mf).
3. Choose your Wonderprint ZR template (.3mf) if not already set.
4. Map colors to toolheads if needed.
5. Click Convert project in the left panel.
6. Open the output in Wonderprint-Orca and re-slice before printing.

Nothing is uploaded — conversion runs only on this PC.
The original project file is never overwritten.

Rebuild (developers)
--------------------
From the repo root:
  powershell -ExecutionPolicy Bypass -File scripts\build-desktop-app.ps1
"@
Set-Content -Path (Join-Path $Dist "README-for-users.txt") -Value $Readme -Encoding UTF8

Write-Host ""
Write-Host "Done. Give non-technical users either:" -ForegroundColor Cyan
Write-Host "  $PortablePath"
if ($Nsis) { Write-Host "  or the setup.exe in dist\" }
Write-Host ""
