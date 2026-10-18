<#
.SYNOPSIS
  Set up a Windows machine to build, test, debug and measure pawnstar-rs: the toolchain, the tools and the editor.

.DESCRIPTION
  Each component is checked first and installed only when it is missing or not the version the repo wants, so the
  script is safe to run again at any time. Everything goes into the user's own profile and needs no administrator,
  except Visual Studio's C++ build tools (Rust's MSVC target links with them), which winget installs behind a UAC
  prompt. tools/setup-dev.sh is the same for Linux and macOS. VS Code's "setup environment" and "check environment"
  tasks run the one for the platform.

  What it sets up:
    - Git for Windows, whose bash runs the UCI shell suite (tests/uci_shell_test.rs)
    - Visual Studio's C++ build tools (the MSVC linker and the Windows SDK)
    - rustup with the stable toolchain (rust-toolchain.toml) and rustfmt, clippy and llvm-tools
    - cargo-llvm-cov, for line coverage
    - the repo's git hooks (core.hooksPath .githooks) and the VS Code extensions .vscode/extensions.json recommends

.PARAMETER Check
  Report what is present and what is missing, install nothing, and exit 1 if anything is missing.

.PARAMETER NoPersist
  Do not add anything to the user's PATH for good (only to this process's): for a test run in a scratch profile.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File tools\setup-dev.ps1           # set up
  powershell -ExecutionPolicy Bypass -File tools\setup-dev.ps1 -Check    # report only
#>
param([switch]$Check, [switch]$NoPersist)
# Continue, not Stop: Windows PowerShell 5.1 makes a native command's stderr a terminating error under Stop, even
# redirected (rustup prints its notes there). The cmdlets that must stop say so; native commands' exit codes are checked.
$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'
Set-Location (Split-Path -Parent $PSScriptRoot)
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

# ---------------------------------------------------------------------------------------------------------------------
# Helpers (the same in every reckless-technology repo's setup script).
$script:missing = 0
function Write-Ok([string]$name, [string]$detail) { Write-Host ("  ok       {0,-24} {1}" -f $name, $detail) -ForegroundColor Green }
function Write-Missing([string]$name, [string]$detail) {
  $script:missing++
  Write-Host ("  missing  {0,-24} {1}" -f $name, $detail) -ForegroundColor Yellow
}
function Write-Installing([string]$name, [string]$detail) { Write-Host ("  install  {0,-24} {1}" -f $name, $detail) -ForegroundColor Cyan }

# Put a directory on this process's PATH and, unless -NoPersist or -Check (which changes nothing), on the user's PATH
# for every new shell.
function Add-ToPath([string]$dir) {
  if (($env:PATH -split ';') -notcontains $dir) { $env:PATH = "$dir;$env:PATH" }
  if ($NoPersist -or $Check) { return }
  $user = [Environment]::GetEnvironmentVariable('Path', 'User')
  if ((($user -split ';') | Where-Object { $_ }) -notcontains $dir) {
    [Environment]::SetEnvironmentVariable('Path', (@($dir) + (($user -split ';') | Where-Object { $_ })) -join ';', 'User')
  }
}

# Download a file and refuse it unless its SHA-256 is the one given.
function Get-VerifiedFile([string]$url, [string]$sha256, [string]$out) {
  Invoke-WebRequest -Uri $url -OutFile $out -UseBasicParsing -ErrorAction Stop
  $got = (Get-FileHash $out -Algorithm SHA256).Hash
  if ($got -ne $sha256.ToUpper()) { Remove-Item $out; throw "SHA-256 mismatch for $url (got $got, want $sha256)" }
}

# A downloaded checksum file's text, whatever type Invoke-WebRequest hands back.
function Get-Text([string]$url) {
  $content = (Invoke-WebRequest -Uri $url -UseBasicParsing -ErrorAction Stop).Content
  if ($content -is [byte[]]) { return [Text.Encoding]::ASCII.GetString($content) }
  return $content
}

function New-ScratchDir {
  $dir = Join-Path ([IO.Path]::GetTempPath()) ("setup-dev-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
  New-Item -ItemType Directory -Path $dir -ErrorAction Stop | Out-Null
  return $dir
}

# Visual Studio (any edition, the Build Tools included) with the x64 C++ tools, found by vswhere.
function Find-VisualStudio([string[]]$components) {
  $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  if (-not (Test-Path $vswhere)) { return $null }
  $found = & $vswhere -latest -products * -requires $components -property installationPath
  if ($found) { return ($found | Select-Object -First 1) }
  return $null
}

# Install Visual Studio Build Tools 2022 with the C++ workload, or add the components to the Visual Studio already
# there. Both need an administrator: winget and the Visual Studio installer raise the UAC prompt themselves.
function Install-VisualStudio([string[]]$components) {
  $add = @('Microsoft.VisualStudio.Workload.VCTools') + $components | ForEach-Object { "--add $_" }
  $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  $existing = if (Test-Path $vswhere) { & $vswhere -latest -products * -property installationPath | Select-Object -First 1 }
  if ($existing) {
    Write-Installing 'Visual Studio' "adding the C++ components to $existing (UAC prompt)"
    $setup = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\setup.exe'
    $p = Start-Process -FilePath $setup -ArgumentList (@('modify', '--installPath', "`"$existing`"", '--passive', '--norestart', '--includeRecommended') + $add) -Wait -PassThru
  }
  else {
    Write-Installing 'Visual Studio' 'Build Tools 2022 with the C++ workload, through winget (UAC prompt)'
    $override = "--wait --passive --norestart --includeRecommended " + ($add -join ' ')
    $p = Start-Process -FilePath winget -ArgumentList @('install', '--id', 'Microsoft.VisualStudio.2022.BuildTools', '-e',
      '--accept-package-agreements', '--accept-source-agreements', '--override', "`"$override`"") -Wait -PassThru -NoNewWindow
  }
  if ($p.ExitCode -notin 0, 3010) { throw "the Visual Studio installer exited with $($p.ExitCode)" }
}

function Set-GitHooks {
  if (-not (Test-Path '.githooks')) { return }
  $current = git config core.hooksPath
  if ($current -eq '.githooks') { Write-Ok 'git hooks' 'core.hooksPath .githooks'; return }
  if ($Check) { Write-Missing 'git hooks' 'core.hooksPath is not .githooks'; return }
  git config core.hooksPath .githooks
  Write-Installing 'git hooks' 'core.hooksPath .githooks'
}

# The extensions .vscode/extensions.json recommends, when VS Code's command line is on the PATH.
function Install-VSCodeExtensions {
  $code = Get-Command code -ErrorAction SilentlyContinue
  if (-not $code -or -not (Test-Path '.vscode/extensions.json')) { return }
  $json = (Get-Content '.vscode/extensions.json' -Raw) -replace '(?m)^\s*//.*$', '' | ConvertFrom-Json
  $installed = & $code.Source --list-extensions 2>$null
  foreach ($ext in $json.recommendations) {
    if ($installed -contains $ext) { Write-Ok 'VS Code extension' $ext; continue }
    if ($Check) { Write-Missing 'VS Code extension' $ext; continue }
    Write-Installing 'VS Code extension' $ext
    & $code.Source --install-extension $ext --force | Out-Null
  }
}

# ---------------------------------------------------------------------------------------------------------------------
Write-Host "pawnstar-rs: $(if ($Check) { 'checking' } else { 'setting up' }) the Windows development environment"

# Git for Windows' bash: the UCI shell suite needs bash >= 4 that reads C:\ paths, never WSL's System32\bash.exe.
$gitBash = $null
$git = Get-Command git -ErrorAction SilentlyContinue
if ($git) {
  $root = (Split-Path (Split-Path (Split-Path (git --exec-path))))
  foreach ($candidate in "$root\bin\bash.exe", "$env:ProgramFiles\Git\bin\bash.exe") {
    if (Test-Path $candidate) { $gitBash = $candidate; break }
  }
}
if ($gitBash) { Write-Ok 'Git for Windows' "$(git --version) ($gitBash)" }
elseif ($Check) { Write-Missing 'Git for Windows' 'git and its bash' }
else {
  Write-Installing 'Git for Windows' 'through winget'
  winget install --id Git.Git -e --accept-package-agreements --accept-source-agreements
  Write-Host '  Git for Windows is installed: open a new terminal and run this script again.'
  exit 0
}

# Visual Studio's C++ build tools.
$vsComponents = @('Microsoft.VisualStudio.Component.VC.Tools.x86.x64')
$vs = Find-VisualStudio $vsComponents
$sdk = Test-Path "${env:ProgramFiles(x86)}\Windows Kits\10\Include\10.*"
if ($vs -and $sdk) { Write-Ok 'Visual Studio C++' $vs }
elseif ($Check) { Write-Missing 'Visual Studio C++' 'the x64 C++ tools and a Windows SDK' }
else { Install-VisualStudio ($vsComponents + 'Microsoft.VisualStudio.Component.Windows11SDK.26100') }

# rustup and the stable toolchain.
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$cargoBin = Join-Path $cargoHome 'bin'
if (Test-Path (Join-Path $cargoBin 'rustup.exe')) { Add-ToPath $cargoBin }
if (Get-Command rustup -ErrorAction SilentlyContinue) { Write-Ok 'rustup' ((rustup --version 2>$null) | Select-Object -First 1) }
elseif ($Check) { Write-Missing 'rustup' 'and the stable toolchain' }
else {
  Write-Installing 'rustup' 'rustup-init from static.rust-lang.org'
  $scratch = New-ScratchDir
  $url = 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe'
  $sha = (Get-Text "$url.sha256").Split(' ')[0].Trim()
  Get-VerifiedFile $url $sha (Join-Path $scratch 'rustup-init.exe')
  & (Join-Path $scratch 'rustup-init.exe') -y --default-host x86_64-pc-windows-msvc --default-toolchain none --no-modify-path | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "rustup-init exited with $LASTEXITCODE" }
  Remove-Item -Recurse -Force $scratch
  Add-ToPath $cargoBin
}

if (Get-Command rustup -ErrorAction SilentlyContinue) {
  $components = 'rustfmt', 'clippy', 'llvm-tools'
  $have = (rustup component list --installed --toolchain stable 2>$null) -join ' '
  $lacking = $components | Where-Object { $have -notmatch "\b$_" }
  if (-not $have) { $lacking = $components }
  if (-not $lacking) { Write-Ok 'Rust stable' "$((rustc +stable --version)) with $($components -join ', ')" }
  elseif ($Check) { Write-Missing 'Rust stable' "needs $($lacking -join ', ')" }
  else {
    Write-Installing 'Rust stable' "with $($components -join ', ')"
    rustup toolchain install stable --profile minimal --component ($components -join ',')
    if ($LASTEXITCODE -ne 0) { throw "rustup exited with $LASTEXITCODE" }
  }

  if (Get-Command cargo-llvm-cov -ErrorAction SilentlyContinue) { Write-Ok 'cargo-llvm-cov' (cargo llvm-cov --version) }
  elseif ($Check) { Write-Missing 'cargo-llvm-cov' 'line coverage' }
  else {
    Write-Installing 'cargo-llvm-cov' 'cargo install --locked'
    cargo +stable install cargo-llvm-cov --locked
    if ($LASTEXITCODE -ne 0) { throw "cargo install exited with $LASTEXITCODE" }
  }
}

Set-GitHooks
Install-VSCodeExtensions

if ($Check) {
  if ($script:missing) { Write-Host "$($script:missing) missing: run tools\setup-dev.ps1 (VS Code: the 'setup environment' task)."; exit 1 }
  Write-Host 'Everything is in place.'
}
else {
  Write-Host 'Done. Open a new terminal (or restart VS Code) for PATH changes to reach it, then: cargo test --release'
}
