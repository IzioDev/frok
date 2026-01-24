$ErrorActionPreference = "Stop"

$BaseUrl = "https://github.com/IzioDev/frok/releases"
$Version = $env:FROK_VERSION
if ($Version) {
  $DownloadBase = "$BaseUrl/download/v$Version"
} else {
  $DownloadBase = "$BaseUrl/latest/download"
}

$arch = "x86_64"
if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") {
  $arch = "arm64"
}

$archive = "frok-windows-$arch.zip"
$tmpDir = Join-Path $env:TEMP "frok-install-$([Guid]::NewGuid().ToString('n'))"
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null

$zipPath = Join-Path $tmpDir $archive
Invoke-RestMethod -Uri "$DownloadBase/$archive" -OutFile $zipPath
Expand-Archive -Path $zipPath -DestinationPath $tmpDir -Force

if ($env:FROK_INSTALL_DIR) {
  $binDir = $env:FROK_INSTALL_DIR
} else {
  $binDir = Join-Path $env:LOCALAPPDATA "frok\\bin"
}

New-Item -ItemType Directory -Force -Path $binDir | Out-Null
Move-Item -Force (Join-Path $tmpDir "frok.exe") (Join-Path $binDir "frok.exe")

Write-Host "installed: $binDir\\frok.exe"
if (Get-Command frok -ErrorAction SilentlyContinue) {
  try { frok --version } catch {}
} else {
  Write-Host "add to PATH: $binDir"
}
